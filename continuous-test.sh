#!/usr/bin/env bash
set -euo pipefail

# continuous-test.sh — stream varied SJBIS notifications for live testing
#
# Usage: ./continuous-test.sh [--count N] [--gap SECONDS] [--once]
#
#   --count N    Number of notifications to send (default: infinite)
#   --gap N      Seconds between posts (default: 3)
#   --once       Send one round of each type then exit
#   --clear      Clear existing data first
#
# Examples:
#   ./continuous-test.sh                  # loop forever, 3s gap
#   ./continuous-test.sh --count 20       # send 20 notifications
#   ./continuous-test.sh --once           # one of each type
#   ./continuous-test.sh --clear --once  # fresh start, one round

API="http://localhost:7878"
COUNT=""
GAP=3
ONCE=false
CLEAR=false
SENT=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --count) COUNT="$2"; shift 2 ;;
        --gap) GAP="$2"; shift 2 ;;
        --once) ONCE=true; shift ;;
        --clear) CLEAR=true; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if $CLEAR; then
    ./clear-test-data.sh --keep-agents || true
fi

function ask() {
    sjbis ask "$@" --json 2>/dev/null
}

function post_yesno() {
    ask \
        --question "Approve expense for team lunch?" \
        --yesno \
        --agent-name "Postmaster" \
        --instance "Expenses" \
        --detail "$47.23 at Thai Garden — 4 people, project kickoff. Dana flagged for approval." \
        --urgency 4
}

function post_text() {
    ask \
        --question "Reply to the design-review thread?" \
        --text \
        --agent-name "Postmaster" \
        --instance "Slack #design" \
        --placeholder 'e.g. "Looks good — ship it."' \
        --detail "Priya asked for final sign-off before merging the Figma branch." \
        --urgency 2
}

function post_numeric() {
    ask \
        --question "How many standup minutes today?" \
        --number \
        --agent-name "Chronos" \
        --instance "Calendar" \
        --min 5 \
        --max 30 \
        --step 5 \
        --default 15 \
        --unit minutes \
        --detail "Standup ran long yesterday (22 min). Try to keep it tight." \
        --urgency 1
}

function post_ack() {
    ask \
        --question "Build succeeded — all 142 tests passed." \
        --ack \
        --agent-name "OpenCode" \
        --instance "CI / main" \
        --detail "No failures, no flaky reruns. Coverage unchanged at 78.4%." \
        --urgency 1
}

function post_multichoice() {
    ask \
        --question "Lunch is coming — pick a cuisine." \
        --choices '[{"value":"thai","label":"Thai","hint":"Pad Thai $12"},{"value":"indian","label":"Indian","hint":"Tandoori $14"},{"value":"salad","label":"Salad","hint":"Sweetgreen $16"},{"value":"skip","label":"Skip it"}]' \
        --agent-name "Hermes" \
        --instance "Lunchbot" \
        --detail "They're ordering in 12 minutes. Last time you skipped and regretted it at 3pm." \
        --urgency 3 \
        --deadline 12m
}

function post_picklist() {
    local tmp
    tmp=$(mktemp)
    cat > "$tmp" <<'EOF'
[
  {"id":"r1","title":"The Thai Garden","meta":"$12 · 0.3mi · ★ 4.6"},
  {"id":"r2","title":"Spice House","meta":"$14 · 0.5mi · ★ 4.8"},
  {"id":"r3","title":"Noodle Bar","meta":"$11 · 0.2mi · ★ 4.4"},
  {"id":"r4","title":"Sweetgreen","meta":"$16 · 0.1mi · ★ 4.3"}
]
EOF
    ask \
        --question "Pick a lunch spot for the team." \
        --pick "$tmp" \
        --agent-name "Tripwise" \
        --instance "Lunchbot" \
        --detail "Team of 4, walking distance preferred. Budget ~$15/head." \
        --urgency 3
    rm -f "$tmp"
}

function post_schedule() {
    local tmp
    tmp=$(mktemp)
    cat > "$tmp" <<'EOF'
[
  {"day":"Mon","time":"10:00 AM"},
  {"day":"Tue","time":"2:00 PM"},
  {"day":"Wed","time":"11:00 AM","disabled":true,"reason":"focus block"},
  {"day":"Thu","time":"9:30 AM"},
  {"day":"Fri","time":"3:00 PM"}
]
EOF
    ask \
        --question "When should I book the 1:1 with Alex?" \
        --schedule "$tmp" \
        --agent-name "Chronos" \
        --instance "Google Calendar" \
        --detail "Alex is free all week except Wednesday mornings. You prefer afternoons." \
        --urgency 2
    rm -f "$tmp"
}

function post_file() {
    ask \
        --question "Upload the signed contract PDF." \
        --file \
        --agent-name "Ledger" \
        --instance "DocuSign" \
        --accept ".pdf" \
        --detail "Legal needs it before EOD to close Q2. Only PDF accepted." \
        --urgency 4 \
        --deadline 4h
}

function post_diff() {
    ask \
        --question "Approve the refactor in PR #412?" \
        --diff \
        --agent-name "OpenCode" \
        --instance "GitHub" \
        --detail "Renames getUser → fetchUser. 14 files, 23 call sites. Tests pass. 2 reviewers approved." \
        --urgency 3
}

TYPES=(yesno text numeric ack multichoice picklist schedule file diff)

# Pre-seed a few agents so they show up with custom glyphs
sjbis register --agent-name "Postmaster" --glyph "✉" --color "oklch(78% 0.18 20)" 2>/dev/null || true
sjbis register --agent-name "Chronos" --glyph "◴" --color "oklch(78% 0.18 200)" 2>/dev/null || true
sjbis register --agent-name "Hermes" --glyph "⚡" --color "oklch(78% 0.18 50)" 2>/dev/null || true
sjbis register --agent-name "Tripwise" --glyph "🍴" --color "oklch(78% 0.18 280)" 2>/dev/null || true
sjbis register --agent-name "OpenCode" --glyph "⚙" --color "oklch(78% 0.18 120)" 2>/dev/null || true
sjbis register --agent-name "Ledger" --glyph "🧾" --color "oklch(78% 0.18 80)" 2>/dev/null || true

echo "═══════════════════════════════════════════════════════════════"
echo "  SJBIS Continuous Test"
echo "  API: $API"
echo "  Gap: ${GAP}s"
if [[ -n "$COUNT" ]]; then
    echo "  Count: $COUNT"
elif $ONCE; then
    echo "  Mode: one round ($((${#TYPES[@]})) types)"
else
    echo "  Mode: infinite loop (Ctrl-C to stop)"
fi
echo "═══════════════════════════════════════════════════════════════"

while true; do
    for t in "${TYPES[@]}"; do
        if [[ -n "$COUNT" && "$SENT" -ge "$COUNT" ]]; then
            echo ""
            echo "Done — sent $SENT notifications."
            exit 0
        fi

        echo ""
        echo "[$((SENT+1))] Posting: $t"
        case "$t" in
            yesno)       post_yesno >/dev/null ;;
            text)        post_text >/dev/null ;;
            numeric)     post_numeric >/dev/null ;;
            ack)         post_ack >/dev/null ;;
            multichoice) post_multichoice >/dev/null ;;
            picklist)    post_picklist >/dev/null ;;
            schedule)    post_schedule >/dev/null ;;
            file)        post_file >/dev/null ;;
            diff)        post_diff >/dev/null ;;
        esac
        SENT=$((SENT+1))
        echo "  ✓ sent ($t)"

        if $ONCE; then
            continue
        fi

        echo "  ...sleeping ${GAP}s..."
        sleep "$GAP"
    done

    if $ONCE; then
        echo ""
        echo "Done — sent one round of all types."
        exit 0
    fi

done
