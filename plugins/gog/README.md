# SJBIS Gog Plugin

Surfaces Gmail and Google Chat messages as SJBIS notifications. Uses AI to classify whether a message is a real question requiring a human response, then routes your answer back to the original sender.

## How it works

1. **Polls Gmail** every 60 seconds for unread threads
2. **Fetches email body** via `gog gmail get` (not just subject/snippet)
3. **AI classification** asks: "Is this a direct question requiring a human response?"
4. **Surfaces** through `sjbis ask`, which posts to the SJBIS daemon
5. **Waits** for you to answer on the dashboard
6. **Sends reply** back via `gog gmail send` with the correct thread
7. **Marks as read** to prevent resurfacing

## Privacy of the `sjbis ask` handoff

When `sjbis-gog` surfaces a Gmail or Google Chat decision, it starts the
blocking `sjbis ask` child with `--content-stdin`. The Gmail/Chat-derived
question and detail bodies are sent as its stdin JSON payload:

```json
{"question":"<message-derived question>","detail":"<message-derived detail>"}
```

They are not passed with `--question` or `--detail`, so embedded links and
token-like text from those bodies do not appear in that child process's
arguments. Its argv retains only non-message invocation metadata: the `ask`
subcommand, `--content-stdin`, answer/blocking/output flags, the configured
agent name, and the profile/source `--instance` value.

This requires an `sjbis` CLI that supports `--content-stdin`; an older CLI will
reject the invocation rather than causing the plugin to fall back to content on
argv. The scope is deliberately narrow: it covers this Gmail/Chat-to-blocking
`sjbis ask` handoff only. It does not claim that other `gog` subprocesses,
reply handling, SJBIS storage or logging, or unrelated plugins have been
audited or secured by this change.

## Prerequisites

1. **Install gog**:
   ```bash
   brew install gog
   ```

2. **Authenticate with Google** (repeat for each profile):
   ```bash
   gog auth login
   # or for a specific profile:
   gog --client=work auth login
   ```

3. **Set up daemon URL**:
   ```bash
   mkdir -p ~/.config/sjbis
   echo 'url = "http://dertog:7878"' > ~/.config/sjbis/daemon.toml
   ```

4. **Set Fireworks API key** (for AI classification):
   ```bash
   export FIREWORKS_API_KEY="fw_..."
   ```

5. **SJBIS daemon running** on the target machine:
   ```bash
   sjbis daemon start
   ```

## Installation

### Build the plugin

```bash
cd plugins/gog
cargo build --release
```

### Cross-compile for Linux (if daemon runs on a server)

```bash
cargo zigbuild --target x86_64-unknown-linux-musl --release
```

### Run as a macOS background service

```bash
# Create the launchd plist
cat > ~/Library/LaunchAgents/com.sjbis.gog.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.sjbis.gog</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Users/USER/dev5/sjbis/plugins/gog/target/release/sjbis-gog</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
        <key>Crashed</key>
        <true/>
    </dict>
    <key>StandardOutPath</key>
    <string>/Users/USER/.config/sjbis/gog.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/USER/.config/sjbis/gog.error.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
        <key>FIREWORKS_API_KEY</key>
        <string>YOUR_FIREWORKS_KEY</string>
    </dict>
</dict>
</plist>
EOF

# Load the service
launchctl load ~/Library/LaunchAgents/com.sjbis.gog.plist

# Check status
launchctl list com.sjbis.gog
```

## Usage

### Run the daemon

```bash
# Auto-detects all authenticated profiles
cd plugins/gog
cargo run --release -- run

# Or with specific profiles
cargo run --release -- run --profile sjbdf --profile work
```

### Test mode (dry run)

```bash
# Test Gmail - shows what would be surfaced without sending
 cargo run --release -- test-gmail --profile sjbdf

# Test Chat - shows what would be surfaced
 cargo run --release -- test-chat --profile sjbdf
```

## AI Classification

The plugin uses Fireworks AI (kimi-k2p6) to determine if an email is a real question:

- **Real questions** → Surfaced to dashboard (e.g., "Are you free?", "What do you think?", "quick question")
- **Newsletters** → Skipped (e.g., The Information Briefing, Substack digests)
- **Marketing** → Skipped (e.g., "EXTRA 25% off", promotional emails)
- **Self-reminders** → Skipped (e.g., "very important message" from yourself)
- **Sales/Spam** → Skipped

If the AI is unavailable, it falls back to regex heuristics.

## Answer Routing

The plugin handles different reply destinations based on the source:

| Source | Reply Method | Format |
|--------|-------------|--------|
| **Gmail** | `gog gmail send --thread-id --reply-all` | Re: Subject |
| **Chat** | `gog chat messages create` | Direct reply |
| **CLI** | `sjbis ask` stdout | JSON with answer |
| **HTTP** | `GET /wait/{id}` | Answer envelope |

After replying, the Gmail thread is **marked as read** to prevent resurfacing.

## Multiple profiles

If you have multiple Google accounts (e.g., personal and work), authenticate each:

```bash
gog --client=personal auth login
gog --client=work auth login
```

The plugin reads `~/.config/gogcli/config.json` to auto-detect all profiles.

## Configuration

The plugin reads `~/.config/sjbis/daemon.toml` for the daemon URL:

```toml
url = "http://dertog:7878"
```

Edit `Config::default()` in `src/main.rs` to customize:
- `poll_interval_secs`: How often to poll (default: 60s)
- `dedup_window_secs`: Dedup window (default: 300s)
- `agent_name`: Name shown in SJBIS dashboard (default: "Gog")
- `profiles`: Specific profiles to monitor (empty = auto-detect all)

## Architecture

```
Gmail API (gog CLI)
    ↓
Fetch unread threads
    ↓
Fetch email body (gog gmail get)
    ↓
AI classifier (Fireworks API)
    → Real question? → POST to SJBIS daemon /ask
    → Newsletter? → Skip
    → Spam? → Skip
    ↓
Dashboard (SJBIS web UI)
    → You answer
    ↓
Reply routed back
    → Gmail: gog gmail send --reply-all
    → Chat: gog chat messages create
    → Mark as read
```

## Logs

```bash
# Service logs
tail -f ~/.config/sjbis/gog.log

# Error logs
tail -f ~/.config/sjbis/gog.error.log
```

## Troubleshooting

### "invalid_grant" error

Your gog token expired. Re-authenticate:
```bash
gog auth login
```

### No profiles found

Check that gog is authenticated:
```bash
gog auth status
```

### AI classifier fails (401 Unauthorized)

Check your Fireworks API key is set:
```bash
echo $FIREWORKS_API_KEY
```

### Gmail replies not sending

Check that `gmail-no-send` is not enabled:
```bash
gog --gmail-no-send=false gmail send --help
```

### Post to daemon fails

Verify the daemon URL is reachable:
```bash
curl http://dertog:7878/health
# or
curl http://192.168.0.138:7878/health
```

## License

MIT
