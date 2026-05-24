#!/usr/bin/env bash
set -euo pipefail

# clear-test-data.sh — wipe all SJBIS Postgres data
#
# Usage: ./clear-test-data.sh [--keep-agents]
#
# With --keep-agents, only notifications and rules are cleared, preserving
# any custom-registered agents. By default, everything is truncated.

KEEP_AGENTS=false
if [[ "${1:-}" == "--keep-agents" ]]; then
    KEEP_AGENTS=true
fi

# Read DSN from config
CONFIG="${HOME}/.config/sjbis/database.toml"
if [[ ! -f "$CONFIG" ]]; then
    echo "❌ Config not found at $CONFIG"
    exit 1
fi

# Extract dsn line from TOML
DSN=$(grep '^dsn ' "$CONFIG" | sed 's/^dsn *= *"//; s/"$//' | head -1)
if [[ -z "$DSN" ]]; then
    echo "❌ Could not read DSN from $CONFIG"
    exit 1
fi

# Check if daemon is running — warn if it is
if command -v sjbis >/dev/null 2>&1; then
    if sjbis daemon status 2>/dev/null | grep -q "running"; then
        echo "⚠️  Warning: sjbis daemon is currently running."
        echo "   Data will be cleared live; the daemon will continue."
        echo ""
    fi
fi

if $KEEP_AGENTS; then
    echo "Clearing notifications and rules (keeping agents)..."
    psql "$DSN" -c "DELETE FROM notifications; DELETE FROM rules;" >/dev/null
    echo "✓ Cleared notifications and rules."
else
    echo "Truncating all tables..."
    psql "$DSN" -c "TRUNCATE notifications, rules, agents RESTART IDENTITY;" >/dev/null
    echo "✓ All tables truncated."
fi

echo ""
echo "Next steps:"
echo "  sjbis daemon start --port 7878   # if not running"
echo "  ./continuous-test.sh             # to stream demo data"
