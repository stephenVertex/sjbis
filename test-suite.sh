#!/usr/bin/env bash
set -eo pipefail

# SJBIS Live Demo — creates notifications with 3–8s gaps for live SSE demo.
# Run this after `sjbis daemon start` is running on port 7878.

API="http://localhost:7878"
DASHBOARD="$API"

echo "═══════════════════════════════════════════════════════════════"
echo "  SJBIS Live Demo"
echo "  Dashboard: $DASHBOARD"
echo "  Gap between notifications: 3–8 seconds"
echo "═══════════════════════════════════════════════════════════════"

# ── Helpers ─────────────────────────────────────────────────────
function ask() {
    sjbis ask "$@" --json 2>/dev/null
}

function ok() {
    echo "  ✓ $1"
}

function gap() {
    local seconds
    seconds=$(python3 -c "import random; print(random.randint(3,8))")
    echo "  ...sleeping ${seconds}s..."
    sleep "$seconds"
}

# ── 1. Expense approval (urgency 5) ────────────────────────────
echo ""
echo "1. Postmaster — urgent expense"
echo "──────────────────────────────"

gap
ID1=$(ask --question "Claim Tuesday's burrito as a business expense?" \
    --yesno \
    --agent-name "Postmaster" \
    --instance "Gmail inbox" \
    --detail "Receipt for $14.20 at Cilantro. Dana flagged it because the meeting with Priya was on calendar — looks deductible. She needs an answer before she files at 5pm." \
    --urgency 5 \
    --deadline 6m \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Urgent yes/no from Postmaster (id: $ID1)"

# ── 2. Family pickup (urgency 4) ─────────────────────────────────
echo ""
echo "2. fam — family logistics"
echo "─────────────────────────"

gap
ID2=$(ask --question "Pick up Mia from soccer at 5?" \
    --yesno \
    --agent-name "fam" \
    --instance "Joey" \
    --detail "Coach texted, practice ends early at 4:45. Wife is on a call until 5:30. If you can't make it, Joey offered to drop her — but you'd owe him a pizza night." \
    --urgency 4 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Family yes/no from fam agent (id: $ID2)"

# ── 3. Code review diff (urgency 3) ──────────────────────────────
echo ""
echo "3. OpenCode — code review"
echo "─────────────────────────"

gap
ID3=$(ask --question "Approve renaming getUser → fetchUser across 14 files?" \
    --yesno \
    --yes-label "Approve & merge" \
    --no-label "Reject — keep getUser" \
    --agent-name "OpenCode" \
    --instance "Session s7b3d11" \
    --detail "Codemod ran clean. 23 callsites updated, 2 tests changed names. No behavior diff in the test run. The PR has been open 3 days and blocks the auth refactor branch." \
    --urgency 3 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Code review yes/no from OpenCode (id: $ID3)"

# ── 4. Clam chowder vote (urgency 4) ─────────────────────────────
echo ""
echo "4. Hermes — multi-choice vote"
echo "─────────────────────────────"

gap
ID4=$(ask --question "Clam chowder vote — they're ordering in 10." \
    --choices '[{"value":"ne","label":"New England","hint":"White, cream"},{"value":"man","label":"Manhattan","hint":"Red, tomato"},{"value":"ri","label":"Rhode Island","hint":"Clear broth"},{"value":"skip","label":"Skip soup"}]' \
    --agent-name "Hermes" \
    --instance "iMessage · Joey" \
    --detail "Joey is at Legal Seafoods with the cousins. They're at the table now, server is coming back. If nobody picks in 8 minutes they're defaulting to NE — but last time they did that, Aunt Pat complained for a week." \
    --urgency 4 \
    --deadline 8m \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "4-option chowder vote from Hermes (id: $ID4)"

# ── 5. Email draft reply (urgency 2) ─────────────────────────────
echo ""
echo "5. Postmaster — free text reply"
echo "───────────────────────────────"

gap
ID5=$(ask --question "One-line reply to the Q3 OKR thread?" \
    --text \
    --agent-name "Postmaster" \
    --instance "Drafts" \
    --placeholder "e.g. 'Yes — 15 min slot works, I'll send slides Friday.'" \
    --detail "Sara asked if you can present the roadmap on the 28th. Thread has 8 people, including the VP. Tone should be warm but brief — she replied to your last email within 2 hours." \
    --urgency 2 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Free text reply request (id: $ID5)"

# ── 6. Grocery order (urgency 2) ─────────────────────────────────
echo ""
echo "6. Shopper — numeric slider"
echo "───────────────────────────"

gap
ID6=$(ask --question "How many cartons of oat milk this week?" \
    --number \
    --agent-name "Shopper" \
    --instance "Instacart" \
    --min 0 \
    --max 8 \
    --step 1 \
    --default 2 \
    --unit cartons \
    --detail "You averaged 2.3 cartons last month. Current price: $4.20. Sale on Oatly ends Thursday. Kids have been drinking more since soccer season started." \
    --urgency 2 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Numeric slider for grocery order (id: $ID6)"

# ── 7. Security ack (urgency 4) ──────────────────────────────────
echo ""
echo "7. Sentinel — security alert"
echo "────────────────────────────"

gap
ID7=$(ask --question "New device signed into your GitHub." \
    --ack \
    --agent-name "Sentinel" \
    --instance "GitHub" \
    --detail "MacBook Pro · San Francisco, CA · 192.0.2.41. Looks like you — same IP as last night. If this wasn't you, the account has admin access to 12 repos including sjbis and openclaw." \
    --urgency 4 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Security ack from Sentinel (id: $ID7)"

# ── 8. File upload (urgency 3) ───────────────────────────────────
echo ""
echo "8. Ledger — file upload"
echo "───────────────────────"

gap
ID8=$(ask --question "Drop the Q1 mileage log here." \
    --file \
    --agent-name "Ledger" \
    --instance "QuickBooks" \
    --accept ".csv,.pdf,.xlsx" \
    --detail "Dana needs the CSV before she finalizes the Schedule C. PDF or CSV works. Last year you forgot and filed an extension — let's not do that again. She leaves at 3pm today." \
    --urgency 3 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "File upload request from Ledger (id: $ID8)"

# ── 9. Diff approval (urgency 3) ───────────────────────────────────
echo ""
echo "9. OpenCode — diff approval"
echo "───────────────────────────"

gap
ID9=$(ask --question "Approve renaming getUser → fetchUser across 14 files?" \
    --diff \
    --agent-name "OpenCode" \
    --instance "Session s7b3d11" \
    --detail "23 callsites updated, 2 tests changed names. No behavior diff in the test run. The auth refactor branch has been blocked on this for 3 days. 3 reviewers approved already, needs your +1 to merge." \
    --urgency 3 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Diff approval from OpenCode (id: $ID9)"

# ── 10. Dentist schedule (urgency 3) ──────────────────────────────
echo ""
echo "10. Chronos — schedule picker"
echo "─────────────────────────────"

SLOTS=$(mktemp)
cat > "$SLOTS" <<'EOF'
[
  {"day":"Thu May 22","time":"2:30 PM"},
  {"day":"Fri May 23","time":"11:00 AM"},
  {"day":"Mon May 26","time":"9:15 AM","disabled":true,"reason":"focus block"},
  {"day":"Tue May 27","time":"3:45 PM"},
  {"day":"Wed May 28","time":"10:30 AM"}
]
EOF

gap
ID10=$(ask --question "When should I book the dentist follow-up?" \
    --schedule "$SLOTS" \
    --agent-name "Chronos" \
    --instance "Google Calendar" \
    --detail "Dr. Wen has 5 openings in the next two weeks. Avoiding your blocked focus mornings (Mon/Wed 9–11am). You missed the last two appointments — they're going to drop you if you miss again." \
    --urgency 3 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Schedule picker from Chronos (id: $ID10)"
rm "$SLOTS"

# ── 11. Hotel picker (urgency 2) ─────────────────────────────────
echo ""
echo "11. Tripwise — pick from list"
echo "─────────────────────────────"

HOTELS=$(mktemp)
cat > "$HOTELS" <<'EOF'
[
  {"id":"h1","title":"The Driskill","meta":"$262 · 0.3mi · ★ 4.7"},
  {"id":"h2","title":"Hotel Saint Cecilia","meta":"$278 · 0.8mi · ★ 4.9"},
  {"id":"h3","title":"Hotel Magdalena","meta":"$241 · 1.1mi · ★ 4.6"},
  {"id":"h4","title":"Carpenter Hotel","meta":"$219 · 1.4mi · ★ 4.5"},
  {"id":"h5","title":"South Congress Hotel","meta":"$255 · 1.0mi · ★ 4.6"},
  {"id":"h6","title":"Hotel Ella","meta":"$232 · 1.6mi · ★ 4.4"},
  {"id":"h7","title":"The LINE Austin","meta":"$268 · 0.5mi · ★ 4.5"},
  {"id":"h8","title":"Austin Proper","meta":"$279 · 0.4mi · ★ 4.7"}
]
EOF

gap
ID11=$(ask --question "Pick a hotel for the Austin trip (Jun 12–14)." \
    --pick "$HOTELS" \
    --agent-name "Tripwise" \
    --instance "Kayak" \
    --detail "Conference is at the Austin Convention Center. Filtered to walkable (under 1.5mi), under $280/night, rating ≥ 4.4. Your usual favorite (Kimpton) is sold out. Need to book by Friday for the corporate rate." \
    --urgency 2 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "Hotel picker from Tripwise (id: $ID11)"
rm "$HOTELS"

# ── 12. Summary ──────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  Demo complete — 11 notifications seeded"
echo "  Dashboard: $DASHBOARD"
echo ""
echo "  Try: click a card → answer → watch SSE update live"
echo "       keyboard nav: J/K navigate, Enter open, 1-9 answer"
echo "═══════════════════════════════════════════════════════════════"

sjbis list
