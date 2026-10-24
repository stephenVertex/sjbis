// sjbis-phone crypto helpers: the ta grant wire format (transient-authority DESIGN.md §4)
// built from WebCrypto. Pure functions, usable from the page and from node for tests.
//
//   canonical bytes  RFC 8785 JCS of the record (our records hold only strings, bools and ints)
//   record_sha256    lowercase hex SHA-256 of the canonical bytes
//   id               "ta-" + first 6 chars of lowercase unpadded base32 of that digest
//   sig              standard base64 of the DER ECDSA P-256/SHA-256 signature over the canonical bytes
//   pubkey           standard base64 of the 65-byte SEC1 uncompressed point
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.SjbisPhoneCrypto = api;
}(typeof self !== 'undefined' ? self : this, function () {
  'use strict';
  var subtle = (typeof crypto !== 'undefined' && crypto.subtle) || (typeof globalThis !== 'undefined' && globalThis.crypto && globalThis.crypto.subtle);
  var B32 = 'abcdefghijklmnopqrstuvwxyz234567';

  // RFC 8785 for the value shapes we produce: sorted keys (UTF-16 order), no whitespace,
  // JSON.stringify escaping (which matches JCS for strings), integers only.
  function jcs(v) {
    if (v === undefined) throw new Error('undefined is not JSON');
    if (v === null || typeof v !== 'object') {
      if (typeof v === 'number' && !Number.isInteger(v)) throw new Error('non-integer numbers are not supported in records');
      return JSON.stringify(v);
    }
    if (Array.isArray(v)) return '[' + v.map(jcs).join(',') + ']';
    var keys = Object.keys(v).filter(function (k) { return v[k] !== undefined; }).sort();
    return '{' + keys.map(function (k) { return JSON.stringify(k) + ':' + jcs(v[k]); }).join(',') + '}';
  }
  function hex(bytes) {
    var s = '';
    for (var i = 0; i < bytes.length; i++) s += (bytes[i] < 16 ? '0' : '') + bytes[i].toString(16);
    return s;
  }
  function base32(bytes) {
    var out = '', bits = 0, value = 0;
    for (var i = 0; i < bytes.length; i++) {
      value = (value << 8) | bytes[i]; bits += 8;
      while (bits >= 5) { out += B32[(value >>> (bits - 5)) & 31]; bits -= 5; }
    }
    if (bits > 0) out += B32[(value << (5 - bits)) & 31];
    return out;
  }
  function b64(bytes) {
    var bin = '';
    for (var i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    return btoa(bin);
  }
  function unb64(s) {
    s = s.replace(/-/g, '+').replace(/_/g, '/');
    while (s.length % 4) s += '=';
    var bin = atob(s), out = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  function b64url(bytes) { return b64(bytes).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''); }
  function utf8(s) { return new TextEncoder().encode(s); }

  // DER SEQUENCE of two INTEGERs from the raw r||s WebCrypto signature.
  function derInt(b) {
    var i = 0;
    while (i < b.length - 1 && b[i] === 0) i++;
    b = b.slice(i);
    if (b[0] & 0x80) { var p = new Uint8Array(b.length + 1); p.set(b, 1); b = p; }
    var out = new Uint8Array(2 + b.length);
    out[0] = 0x02; out[1] = b.length; out.set(b, 2);
    return out;
  }
  function derSig(raw) {
    if (raw.length !== 64) throw new Error('raw ECDSA signature must be 64 bytes');
    var r = derInt(raw.slice(0, 32)), s = derInt(raw.slice(32));
    var out = new Uint8Array(2 + r.length + s.length);
    out[0] = 0x30; out[1] = r.length + s.length; out.set(r, 2); out.set(s, 2 + r.length);
    return out;
  }
  // The reverse, for verifying an envelope with WebCrypto (tests, self-check).
  function rawSig(der) {
    if (der[0] !== 0x30) throw new Error('not a DER sequence');
    var i = 2, out = new Uint8Array(64);
    for (var k = 0; k < 2; k++) {
      if (der[i] !== 0x02) throw new Error('not a DER integer');
      var len = der[i + 1], start = i + 2, v = der.slice(start, start + len);
      while (v.length > 32 && v[0] === 0) v = v.slice(1);
      out.set(v, k * 32 + (32 - v.length));
      i = start + len;
    }
    return out;
  }

  function iso(d) { return d.toISOString().replace(/\.\d{3}Z$/, 'Z'); }

  async function generateKeyPair() {
    return subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
  }
  async function exportPubkey(publicKey) {
    var raw = new Uint8Array(await subtle.exportKey('raw', publicKey));
    if (raw.length !== 65 || raw[0] !== 4) throw new Error('unexpected public key encoding');
    return b64(raw);
  }

  // The unsigned grant record for an ask, exactly as `ta approve` builds it: the question verbatim,
  // optional fields omitted when absent, timestamps at second precision.
  function grantRecord(ask, opts) {
    var now = opts.now || new Date();
    var rec = {
      v: 1, kind: 'grant', ask_id: ask.id, asked_by: ask.agent_name || '', question: ask.question,
      decision: opts.decision, issued_at: iso(now), expires_at: iso(new Date(now.getTime() + opts.ttlSecs * 1000)),
      signer: opts.signer,
    };
    if (opts.answer != null && opts.answer !== '') rec.answer = opts.answer;
    if (opts.note) rec.note = opts.note;
    if (opts.once) rec.once = true;
    if (ask.renews) rec.renews = ask.renews;
    return rec;
  }
  async function signRecord(record, privateKey) {
    var canonical = utf8(jcs(record));
    var digest = new Uint8Array(await subtle.digest('SHA-256', canonical));
    var raw = new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, privateKey, canonical));
    return { id: 'ta-' + base32(digest).slice(0, 6), record: record, record_sha256: hex(digest), alg: 'es256', sig: b64(derSig(raw)) };
  }
  async function verifyEnvelope(env, pubkeyB64) {
    var canonical = utf8(jcs(env.record));
    var digest = new Uint8Array(await subtle.digest('SHA-256', canonical));
    if (hex(digest) !== env.record_sha256) return 'hash mismatch';
    if ('ta-' + base32(digest).slice(0, 6) !== env.id) return 'id mismatch';
    var key = await subtle.importKey('raw', unb64(pubkeyB64), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
    var ok = await subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, rawSig(unb64(env.sig)), canonical);
    return ok ? null : 'bad signature';
  }

  return { jcs: jcs, hex: hex, base32: base32, b64: b64, unb64: unb64, b64url: b64url, derSig: derSig, rawSig: rawSig, iso: iso,
    generateKeyPair: generateKeyPair, exportPubkey: exportPubkey, grantRecord: grantRecord, signRecord: signRecord, verifyEnvelope: verifyEnvelope };
}));
