# SJBIS Gog Plugin

Surfaces Gmail and Google Chat messages as SJBIS notifications.

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

3. **SJBIS daemon running** (the plugin shells out to `sjbis ask`):
   ```bash
   sjbis daemon start
   ```

## Usage

### Run the daemon

```bash
cd plugins/gog
cargo run -- run
```

This polls all authenticated gog profiles for:
- **Unread Gmail threads** that look like questions/requests
- **Google Chat messages** that look like questions/requests

When it finds one, it runs `sjbis ask --blocking` and waits for you to answer on the dashboard. When you answer, it sends the reply back via Gmail or Chat.

### Test mode (dry run)

```bash
# Test Gmail
 cargo run -- test-gmail --profile default

# Test Chat
 cargo run -- test-chat --profile default
```

### Test a specific profile

```bash
cargo run -- run
# Auto-detects all authenticated profiles
```

## How it works

1. **Polls Gmail** every 60 seconds:
   ```bash
   gog gmail search "is:unread newer_than:1d" -j
   ```
   Filters threads by the same question heuristic as iMessage.

2. **Polls Chat** every 60 seconds:
   ```bash
   gog chat spaces list -j
   gog chat messages list <space> -j
   ```
   Checks messages in all spaces for question-like content.

3. **Surfaces** via HTTP POST to the SJBIS daemon:
   ```bash
   POST http://dertog:7878/ask
   {"question": "...", "agent_name": "Gog", "instance": "profile · sender", "blocking": true}
   ```
   Then blocks on `GET /wait/{id}` until you answer on the dashboard.

4. **Sends reply** when you answer:
   ```bash
   gog gmail send --reply-to <thread_id> --body "..."
   gog chat messages create <space> --text "..."
   ```

## Multiple profiles

If you have multiple Google accounts (e.g., personal and work), authenticate each:

```bash
gog --client=personal auth login
gog --client=work auth login
```

The plugin auto-detects all profiles and polls each one independently.

## Configuration

### Daemon URL

The plugin reads the daemon URL from `~/.config/sjbis/daemon.toml`:

```toml
url = "http://dertog:7878"
```

Create this file so the plugin knows where to POST notifications:

```bash
mkdir -p ~/.config/sjbis
echo 'url = "http://dertog:7878"' > ~/.config/sjbis/daemon.toml
```

### Other options

Edit `Config::default()` in `src/main.rs` to customize:
- `poll_interval_secs`: How often to poll (default: 60s)
- `dedup_window_secs`: Dedup window (default: 300s)
- `agent_name`: Name shown in SJBIS dashboard (default: "Gog")
- `profiles`: Specific profiles to monitor (empty = auto-detect all)

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

### Gmail replies not sending

Check that `gmail-no-send` is not enabled:
```bash
gog --gmail-no-send=false gmail send --help
```

## License

MIT
