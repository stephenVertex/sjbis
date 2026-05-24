#!/usr/bin/env bash
set -euo pipefail

# poll-for-answers.sh — ask 10 non-blocking questions, then poll until answered

API="http://localhost:7878"
MAX_WAIT=120   # seconds
INTERVAL=10    # seconds

echo "═══════════════════════════════════════════════════════════════"
echo "  SJBIS Non-blocking Batch + Poll"
echo "  Dashboard: $API"
echo "  Asking 10 questions, then polling every ${INTERVAL}s for ${MAX_WAIT}s"
echo "═══════════════════════════════════════════════════════════════"

# ── Helpers ─────────────────────────────────────────────────────
function ask() {
    sjbis ask "$@" --json 2>/dev/null
}

function get_id() {
    python3 -c "import sys,json; print(json.load(sys.stdin)['id'])"
}

function get_status() {
    python3 -c "import sys,json; print(json.load(sys.stdin)['status'])"
}

# ── Seed 10 questions (fire-and-forget) ─────────────────────────
declare -a IDS

echo ""
echo "Asking 10 questions..."

IDS[0]=$(ask --question "Approve expense for team lunch?" \
    --yesno \
    --agent-name "Postmaster" \
    --instance "Expenses" \
    --detail "\$47.23 at Thai Garden — 4 people, project kickoff." \
    --urgency 4 \
    | get_id)

IDS[1]=$(ask --question "Reply to the design-review thread?" \
    --text \
    --agent-name "Postmaster" \
    --instance "Slack #design" \
    --placeholder "e.g. 'Looks good — ship it.'" \
    --detail "Priya asked for final sign-off before merging the Figma branch." \
    --urgency 2 \
    | get_id)

IDS[2]=$(ask --question "How many extra standup minutes today?" \
    --number \
    --agent-name "Chronos" \
    --instance "Calendar" \
    --min 5 --max 30 --step 5 --default 15 --unit minutes \
    --detail "Standup ran long yesterday (22 min). Try to keep it tight." \
    --urgency 1 \
    | get_id)

IDS[3]=$(ask --question "Build succeeded — all 142 tests passed." \
    --ack \
    --agent-name "OpenCode" \
    --instance "CI / main" \
    --detail "No failures, no flaky reruns. Coverage unchanged at 78.4%." \
    --urgency 1 \
    | get_id)

IDS[4]=$(ask --question "Lunch is coming — pick a cuisine." \
    --choices '[{"value":"thai","label":"Thai Garden","hint":"$12 · 0.3mi"},{"value":"indian","label":"Spice House","hint":"$14 · 0.5mi"},{"value":"salad","label":"Sweetgreen","hint":"$16 · 0.1mi"},{"value":"skip","label":"Skip lunch"}]' \
    --agent-name "Hermes" \
    --instance "Lunchbot" \
    --detail "Team of 4, ordering in 10 minutes." \
    --urgency 3 \
    --deadline 12m \
    | get_id)

TMP_PICK=$(mktemp)
cat > "$TMP_PICK" <<'EOF'
[
  {"id":"r1","title":"The Thai Garden","meta":"$12 · 0.3mi · ★ 4.6"},
  {"id":"r2","title":"Spice House","meta":"$14 · 0.5mi · ★ 4.8"},
  {"id":"r3","title":"Noodle Bar","meta":"$11 · 0.2mi · ★ 4.4"},
  {"id":"r4","title":"Sweetgreen","meta":"$16 · 0.1mi · ★ 4.3"}
]
EOF
IDS[5]=$(ask --question "Pick a lunch spot for the team." \
    --pick "$TMP_PICK" \
    --agent-name "Tripwise" \
    --instance "Lunchbot" \
    --detail "Team of 4, walking distance preferred. Budget ~$15/head." \
    --urgency 3 \
    | get_id)
rm -f "$TMP_PICK"

TMP_SCHED=$(mktemp)
cat > "$TMP_SCHED" <<'EOF'
[
  {"day":"Mon","time":"10:00 AM"},
  {"day":"Tue","time":"2:00 PM"},
  {"day":"Wed","time":"11:00 AM","disabled":true,"reason":"focus block"},
  {"day":"Thu","time":"9:30 AM"},
  {"day":"Fri","time":"3:00 PM"}
]
EOF
IDS[6]=$(ask --question "When should I book the 1:1 with Alex?" \
    --schedule "$TMP_SCHED" \
    --agent-name "Chronos" \
    --instance "Google Calendar" \
    --detail "Alex is free all week except Wednesday mornings. You prefer afternoons." \
    --urgency 2 \
    | get_id)
rm -f "$TMP_SCHED"

IDS[7]=$(ask --question "Upload the signed contract PDF." \
    --file \
    --agent-name "Ledger" \
    --instance "DocuSign" \
    --accept ".pdf" \
    --detail "Legal needs it before EOD to close Q2. Only PDF accepted." \
    --urgency 4 \
    --deadline 4h \
    | get_id)

IDS[8]=$(ask --question "Approve the refactor in PR #412?" \
    --diff \
    --agent-name "OpenCode" \
    --instance "GitHub" \
    --detail "Renames getUser → fetchUser. 14 files, 23 call sites. Tests pass. 2 reviewers approved." \
    --urgency 3 \
    | get_id)

IDS[9]=$(ask --question "Claim Tuesday's burrito as a business expense?" \
    --yesno \
    --yes-label "Approve & merge" \
    --no-label "Reject — keep getUser" \
    --agent-name "Postmaster" \
    --instance "Gmail inbox" \
    --detail "Receipt for $14.20 at Cilantro. Dana flagged it because the meeting with Priya was on calendar — looks deductible." \
    --urgency 5 \
    --deadline 6m \
    | get_id)

echo ""
echo "Posted 10 notifications:"
for i in "${!IDS[@]}"; do
    echo "  [$((i+1))] ${IDS[$i]}"
done

# ── Poll loop ────────────────────────────────────────────────────
echo ""
echo "Polling every ${INTERVAL}s for up to ${MAX_WAIT}s..."
echo "Open http://localhost:7878 to answer."
echo ""

ELAPSED=0
ANSWERED=0

while [[ "$ELAPSED" -lt "$MAX_WAIT" && "$ANSWERED" -lt 10 ]]; do
    sleep "$INTERVAL"
    ELAPSED=$((ELAPSED + INTERVAL))

    # Fetch all open notifications
    OPEN_JSON=$(sjbis list --json 2>/dev/null || echo "[]")
    OPEN_COUNT=$(echo "$OPEN_JSON" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))")
    ANSWERED=$((10 - OPEN_COUNT))

    echo "  [${ELAPSED}s] ${ANSWERED}/10 answered, ${OPEN_COUNT} still open"

    # Show which ones are still open
    if [[ "$OPEN_COUNT" -gt 0 ]]; then
        echo "$OPEN_JSON" | python3 -c "
import sys, json
for n in json.load(sys.stdin):
    print(f\"    - [{n['id']}] {n['agent_name']}: {n['question']}\")
" || true
    fi
done

# ── Final results ──────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  Final Results"
echo "═══════════════════════════════════════════════════════════════"

# Re-check each ID
ALL_ANSWERED=true
for id in "${IDS[@]}"; do
    STATUS=$(sjbis list --json 2>/dev/null | python3 -c "
import sys, json
for n in json.load(sys.stdin):
    if n['id'] == '$id':
        print('open')
        break
else:
    print('answered')
" || echo "unknown")
    if [[ "$STATUS" == "answered" ]]; then
        echo "  ✓ $id — answered"
    else
        echo "  ○ $id — still open"
        ALL_ANSWERED=false
    fi
done

if $ALL_ANSWERED; then
    echo ""
    echo "All 10 questions answered! 🎉"
else
    echo ""
    echo "Some questions remain unanswered. Check the dashboard."
fi
