# SJBIS iMessage Plugin

Surface iMessage questions to your SJBIS dashboard automatically. This plugin polls your macOS Messages database, detects which texts look like questions, and posts them to the SJBIS surfacer so you can answer from the keyboard-centric web dashboard.

## What It Does

- **Reads** your iMessage history from `~/Library/Messages/chat.db`
- **Filters** out noise ("lol", "ok", "on my way", emojis, etc.)
- **Detects** actual questions ("Can you pick me up?", "Does 10:30 still work?")
- **Surfaces** them to `http://localhost:7878` as SJBIS notifications
- **Dedupes** so the same text doesn't spam the dashboard

## Prerequisites

- macOS (this plugin uses the Messages app database)
- SJBIS daemon running on `http://localhost:7878`
- Full Disk Access permission for your terminal app
- Automation permission for the plugin to interact with Messages

## Building

```bash
cd /Users/stephen/dev5/sjbis/plugins/imessage
cargo build --release
```

The binary will be at:
```
target/release/sjbis-imessage
```

## Setup

### 1. Grant Full Disk Access

The plugin needs to read `~/Library/Messages/chat.db` which is protected by macOS.

1. Open **System Settings → Privacy & Security → Full Disk Access**
2. Click the **+** button
3. Add your **terminal app** (e.g. Terminal.app, iTerm, Warp)
4. Make sure the toggle is **ON** (blue)
5. **Restart your terminal app** if it was already running

Verify access works:
```bash
sqlite3 ~/Library/Messages/chat.db "SELECT text FROM message LIMIT 3;"
```

If you see messages, you're good. If you see "authorization denied", the permission isn't active yet.

### 2. Grant Automation Permission

The plugin uses AppleScript to check if the Messages app is running.

When you first run the plugin, macOS will show a dialog:
> **"sjbis-imessage wants to control Messages"**

Click **Allow**.

If you missed the dialog:
1. Open **System Settings → Privacy & Security → Automation**
2. Find your terminal app in the list
3. Make sure **Messages** is checked underneath it

### 3. Strip Security Attributes (if binary is killed)

macOS may quarantine the binary with `com.apple.provenance` or `com.apple.quarantine` flags.

If running the binary shows `[1] 67240 killed`, remove the flags:

```bash
xattr -d com.apple.provenance /Users/stephen/dev5/sjbis/plugins/imessage/target/release/sjbis-imessage 2>/dev/null
xattr -d com.apple.quarantine /Users/stephen/dev5/sjbis/plugins/imessage/target/release/sjbis-imessage 2>/dev/null
```

## Running

### Test Mode (dry run)

See what would be surfaced without posting to SJBIS:

```bash
cd /Users/stephen/dev5/sjbis/plugins/imessage
./target/release/sjbis-imessage test --minutes 60
```

Output shows:
- Total messages in the time window
- `[skip]` for non-questions
- `[QUESTION]` for detected questions with inferred choices

Use `--minutes 43200` (30 days) for the first run to see the full scope.

### Daemon Mode (live)

Post new iMessage questions to the SJBIS dashboard in real-time:

```bash
cd /Users/stephen/dev5/sjbis/plugins/imessage
./target/release/sjbis-imessage run
```

- Polls every 5 seconds
- Only surfaces **new** messages since the last poll
- Keeps a 5-minute dedup window to prevent re-surfacing

**Keep the terminal open** while running. If you close it, the plugin stops.

## Question Detection

The plugin uses a heuristic filter to decide what to surface:

### Detected as questions
- Contains `?` ("Are you coming?")
- Starts with question words: `what`, `when`, `where`, `why`, `how`, `can you`, `could you`, `would you`, `did you`, `do you`, `are you`, `should i`, `shall we`
- Contains request patterns: `let me know`, `what do you think`, `please confirm`, `confirm`, `approve`, `yes or no`

### Filtered out (noise)
- Very short (< 5 chars)
- Reaction words: `lol`, `haha`, `ok`, `okay`, `k`, `thanks`, `ty`, `np`, `sure`, `nice`, `cool`, `wow`, `brb`, `gtg`
- Non-questions: `on my way`, `omw`, `be there`, `see you`, `sounds good`, `got it`, `will do`, `done`

### Choice inference
The plugin tries to detect multiple-choice options from text:
- `"A or B"` patterns → surfaces as `--choices`
- Comma-separated short options (2–5 items, < 20 chars each)
- Numbered lists (e.g., `1) option 2) option`)

## Architecture

```
iMessage DB (SQLite)
    ↓
DB Poller (every 5s)
    ↓
Question Filter (heuristic)
    ↓
Dedup Cache (5-min window)
    ↓
sjbis ask --blocking
    ↓
SJBIS Dashboard (localhost:7878)
    ↓
Human answers (Yes/No/Note/Snooze)
```

## Files

- `src/main.rs` — CLI entrypoint, daemon loop, dedup cache
- `src/db_poller.rs` — SQLite DB access, Apple epoch date conversion
- `src/question_filter.rs` — Question detection heuristic, choice inference
- `src/observer.rs` — AppleScript notification poller (fallback)
- `src/jxa.rs` — JavaScript for Automation message fetcher
- `src/applescript.rs` — AppleScript message fetcher + reply sender
- `bundle/sjbis-imessage.app/` — Signed .app bundle (alternative to raw binary)

## Troubleshooting

### "unable to open database file"

Full Disk Access is not active. Check:
1. Is your terminal app in the Full Disk Access list?
2. Is the toggle ON (blue)?
3. Did you restart the terminal after granting?
4. Try `sqlite3 ~/Library/Messages/chat.db ".tables"` — if this fails, the permission isn't working

### Binary is killed immediately (`[1] 67240 killed`)

macOS Gatekeeper is terminating the binary. Remove quarantine:
```bash
xattr -c /Users/stephen/dev5/sjbis/plugins/imessage/target/release/sjbis-imessage
```

### "No messages found"

Try a larger time window:
```bash
./target/release/sjbis-imessage test --minutes 43200  # 30 days
```

Messages may have been deleted from the DB or are older than your window.

### "Invalid column type" errors

The DB schema changed (Apple updates macOS). Check the schema:
```bash
sqlite3 ~/Library/Messages/chat.db ".schema message"
```

Update the SQL query in `src/db_poller.rs` if columns changed.

### Automation permission denied

If the plugin can't check if Messages is running:
1. System Settings → Privacy & Security → Automation
2. Find your terminal app
3. Check the box next to **Messages**

## Future Work

- [ ] Background daemon mode (no open terminal required)
- [ ] Send replies back to iMessage contacts via AppleScript/JXA
- [ ] macOS NSDistributedNotificationCenter observer (push instead of poll)
- [ ] Better choice inference (handle more conversational patterns)
- [ ] Contact name resolution (map phone numbers to Contact.app names)
- [ ] Config file (custom question patterns, dedup window, poll interval)
