"""Short-lived message previews for the local Chromium kiosk."""

import argparse
import base64
import json
import socket
import time
from http.server import BaseHTTPRequestHandler, HTTPServer


MAX_BODY = 128 * 1024
RETENTION_SECONDS = 60
MAX_PREVIEWS = 12


class PreviewServer(HTTPServer):
    def __init__(self, origin):
        super().__init__(("127.0.0.1", 7881), PreviewHandler)
        self.origin = origin
        self.previews = {}

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(4)
        return connection, address

    def prune(self):
        cutoff = time.monotonic() - RETENTION_SECONDS
        self.previews = {
            key: (arrival, message) for key, (arrival, message) in self.previews.items()
            if arrival > cutoff
        }

    def service_actions(self):
        self.prune()


class PreviewHandler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, status, value):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", len(body))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Vary", "Origin")
        if self.headers.get("Origin") == self.server.origin:
            self.send_header("Access-Control-Allow-Origin", self.server.origin)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path != "/messages":
            return self.respond(404, {})
        if self.headers.get("Origin") not in (None, self.server.origin):
            return self.respond(403, {})
        self.server.prune()
        return self.respond(200, [message for _, message in self.server.previews.values()])

    def do_POST(self):
        if self.path != "/messages":
            return self.respond(404, {})
        if self.headers.get("Origin") or self.headers.get_content_type() != "application/json":
            return self.respond(403, {})
        try:
            size = int(self.headers.get("Content-Length", "0"))
            if not 0 < size <= MAX_BODY:
                return self.respond(413, {})
            message = validate(json.loads(self.rfile.read(size)))
        except (ValueError, TypeError, socket.timeout):
            return self.respond(400, {})

        self.server.prune()
        if message["id"] not in self.server.previews:
            if len(self.server.previews) >= MAX_PREVIEWS:
                del self.server.previews[next(iter(self.server.previews))]
            self.server.previews[message["id"]] = (time.monotonic(), message)
        return self.respond(200, {"ok": True})


def validate(message):
    if not isinstance(message, dict):
        raise ValueError("Expected a preview object")
    result = {}
    for field, limit in (("id", 200), ("name", 80), ("text", 600)):
        value = message.get(field)
        if not isinstance(value, str) or not value.strip() or len(value) > limit:
            raise ValueError(f"Invalid {field}")
        result[field] = value
    photo = message.get("photo")
    if photo is not None:
        if not isinstance(photo, str) or not base64.b64decode(photo, validate=True).startswith(b"\xff\xd8\xff"):
            raise ValueError("Expected a JPEG thumbnail")
        result["photo"] = photo
    return result


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--origin", required=True, help="Origin of the kiosk page")
    with PreviewServer(cli.parse_args().origin) as server:
        server.serve_forever()
