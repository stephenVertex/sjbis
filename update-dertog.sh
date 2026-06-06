#!/usr/bin/env bash
#
# update-dertog.sh — update the dertog daemon to the latest GitHub release.
#
# This is the release-driven counterpart to build-and-deploy.sh. Instead of
# building from your local working tree and scp'ing, it pulls the published
# release that `sjbis upgrade` reads from:
#
#   1. Run `sjbis upgrade` on the remote (self-replaces the binary from the
#      latest GitHub release; --tag pins a specific release).
#   2. Refresh the bundled static/ dashboard assets from the same release
#      tarball (`sjbis upgrade` only swaps the binary, not static assets).
#   3. Restart the daemon.
#   4. Status checks (health, version, dashboard reachability).
#
# Prereqs: the remote already has a `sjbis` binary new enough to have the
# `upgrade` subcommand (>= 0.1.2). For a first install on an older box, use
# build-and-deploy.sh once, then this script thereafter.
#
# Usage:
#   ./update-dertog.sh                 # update to the latest release
#   ./update-dertog.sh v0.1.3          # update to a specific tag
#
set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────────
REMOTE_HOST="${SJBIS_REMOTE_HOST:-dertog}"
REMOTE_DIR="${SJBIS_REMOTE_DIR:-~/sjbis}"
REMOTE_URL="${SJBIS_REMOTE_URL:-http://dertog:7878}"
REMOTE_BIN="${SJBIS_REMOTE_BIN:-${REMOTE_DIR}/sjbis}"
SERVICE="sjbis"
REPO="stephenVertex/sjbis"
TARGET="x86_64-unknown-linux-musl"
TAG="${1:-}"   # optional: a specific release tag like v0.1.3

# ── Pretty logging ──────────────────────────────────────────────────────
bold() { printf '\033[1m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '  \033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

if [ -n "$TAG" ]; then
  bold "sjbis update ${REMOTE_HOST}  →  release ${TAG}"
else
  bold "sjbis update ${REMOTE_HOST}  →  latest release"
fi

# ── 1. Self-upgrade the binary on the remote ─────────────────────────────
bold "1/4  Upgrade binary on ${REMOTE_HOST}"
UPGRADE_CMD="${REMOTE_BIN} upgrade"
[ -n "$TAG" ] && UPGRADE_CMD="${UPGRADE_CMD} --tag ${TAG}"
ssh "$REMOTE_HOST" "$UPGRADE_CMD" || die "remote 'sjbis upgrade' failed"

# ── 2. Refresh static dashboard assets from the release tarball ──────────
bold "2/4  Refresh static assets from release"
ASSET="sjbis-${TARGET}.tar.gz"
if [ -n "$TAG" ]; then
  REL_URL="https://api.github.com/repos/${REPO}/releases/tags/${TAG}"
else
  REL_URL="https://api.github.com/repos/${REPO}/releases/latest"
fi
# Find the asset's download URL, pull the tarball, extract just static/* into place.
ssh "$REMOTE_HOST" "set -e
  url=\$(curl -fsSL '${REL_URL}' | grep -oE 'https://[^\"]*${ASSET}' | head -1)
  [ -n \"\$url\" ] || { echo 'could not find ${ASSET} on release'; exit 1; }
  tmp=\$(mktemp -d)
  curl -fsSL \"\$url\" -o \"\$tmp/${ASSET}\"
  tar -xzf \"\$tmp/${ASSET}\" -C \"\$tmp\"
  if [ -d \"\$tmp/static\" ]; then
    mkdir -p ${REMOTE_DIR}/static
    cp -f \"\$tmp\"/static/* ${REMOTE_DIR}/static/
  fi
  rm -rf \"\$tmp\"
" && ok "static assets refreshed" || warn "static refresh skipped/failed (binary still updated)"

# ── 3. Restart daemon ────────────────────────────────────────────────────
bold "3/4  Restart ${SERVICE} on ${REMOTE_HOST}"
ssh "$REMOTE_HOST" "systemctl --user restart ${SERVICE}" || die "restart failed"
sleep 2
STATE="$(ssh "$REMOTE_HOST" "systemctl --user is-active ${SERVICE}" 2>/dev/null || true)"
[ "$STATE" = "active" ] && ok "service active" || die "service not active (state: ${STATE})"

# ── 4. Status checks ─────────────────────────────────────────────────────
bold "4/4  Status checks"

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

# dashboard reachable
CODE="$(curl -s -o /dev/null -w '%{http_code}' -m 5 "${REMOTE_URL}/" 2>/dev/null || echo 000)"
[ "$CODE" = "200" ] && ok "dashboard reachable (HTTP ${CODE})" || warn "dashboard HTTP ${CODE}"

bold "Done. ${REMOTE_URL} is running ${REMOTE_VER}"
