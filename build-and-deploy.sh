#!/usr/bin/env bash
#
# build-and-deploy.sh — build, install locally, and deploy sjbis to dertog.
#
# Steps:
#   1. Build + globally install the local sjbis (native).
#   2. Cross-compile a static x86_64 Linux binary.
#   3. scp the binary + static assets to dertog and swap them in.
#   4. Restart the daemon on dertog.
#   5. Status checks (health, version match, dashboard reachability).
#
# Usage: ./build-and-deploy.sh
#
set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────────
REMOTE_HOST="${SJBIS_REMOTE_HOST:-dertog}"
REMOTE_DIR="${SJBIS_REMOTE_DIR:-~/sjbis}"
REMOTE_URL="${SJBIS_REMOTE_URL:-http://dertog:7878}"
TARGET="x86_64-unknown-linux-musl"
SERVICE="sjbis"

# ── Pretty logging ──────────────────────────────────────────────────────
bold() { printf '\033[1m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '  \033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

cd "$(dirname "$0")"

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"
GIT_HASH="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then GIT_HASH="${GIT_HASH}-dirty"; fi
EXPECTED="${VERSION}+${GIT_HASH}"

bold "sjbis build & deploy  →  ${EXPECTED}"

# ── 1. Build + install locally ──────────────────────────────────────────
bold "1/5  Build + install local sjbis"
cargo install --path . --force >/dev/null 2>&1 || die "local cargo install failed"
LOCAL_VER="$(sjbis --version 2>/dev/null | awk '{print $2}')"
ok "installed $(command -v sjbis) → ${LOCAL_VER}"
[ "$LOCAL_VER" = "$EXPECTED" ] || warn "local version ${LOCAL_VER} != expected ${EXPECTED}"

# ── 2. Cross-compile static Linux binary ────────────────────────────────
bold "2/5  Cross-compile static ${TARGET}"
cargo zigbuild --target "$TARGET" --release >/dev/null 2>&1 || die "zigbuild failed"
BIN="target/${TARGET}/release/sjbis"
[ -f "$BIN" ] || die "binary not found at ${BIN}"
ok "built ${BIN} ($(du -h "$BIN" | awk '{print $1}'))"

# ── 3. Copy binary + static assets to dertog ────────────────────────────
bold "3/5  Deploy to ${REMOTE_HOST}"
scp -q "$BIN" "${REMOTE_HOST}:${REMOTE_DIR}/sjbis.new" || die "scp binary failed"
scp -q static/*.jsx static/*.css static/*.html "${REMOTE_HOST}:${REMOTE_DIR}/static/" \
  || die "scp static assets failed"
ssh "$REMOTE_HOST" "mv ${REMOTE_DIR}/sjbis ${REMOTE_DIR}/sjbis.bak 2>/dev/null; \
  mv ${REMOTE_DIR}/sjbis.new ${REMOTE_DIR}/sjbis && chmod +x ${REMOTE_DIR}/sjbis" \
  || die "remote binary swap failed"
ok "binary + static assets copied; previous binary kept as sjbis.bak"

# ── 4. Restart daemon ───────────────────────────────────────────────────
bold "4/5  Restart ${SERVICE} on ${REMOTE_HOST}"
ssh "$REMOTE_HOST" "systemctl --user restart ${SERVICE}" || die "restart failed"
sleep 2
STATE="$(ssh "$REMOTE_HOST" "systemctl --user is-active ${SERVICE}" 2>/dev/null || true)"
[ "$STATE" = "active" ] && ok "service active" || die "service not active (state: ${STATE})"

# ── 5. Status checks ────────────────────────────────────────────────────
bold "5/5  Status checks"

# wait for /health
for i in $(seq 1 10); do
  if [ "$(curl -s -m 3 "${REMOTE_URL}/health" 2>/dev/null)" = "ok" ]; then break; fi
  sleep 1
done
[ "$(curl -s -m 3 "${REMOTE_URL}/health" 2>/dev/null)" = "ok" ] || die "health check failed"
ok "health: ok"

REMOTE_VER="$(curl -s -m 5 "${REMOTE_URL}/version" 2>/dev/null \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["full"])' 2>/dev/null || echo "?")"
ok "remote version: ${REMOTE_VER}"
if [ "$REMOTE_VER" = "$EXPECTED" ]; then
  ok "version matches expected (${EXPECTED})"
else
  warn "remote ${REMOTE_VER} != expected ${EXPECTED}"
fi

# dashboard reachable
CODE="$(curl -s -o /dev/null -w '%{http_code}' -m 5 "${REMOTE_URL}/" 2>/dev/null || echo 000)"
[ "$CODE" = "200" ] && ok "dashboard reachable (HTTP ${CODE})" || warn "dashboard HTTP ${CODE}"

bold "Done. ${REMOTE_URL} is running ${REMOTE_VER}"
