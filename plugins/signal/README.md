# SJBIS Signal Plugin

Surfaces Signal messages as SJBIS notifications using `signal-cli`. Uses AI to classify whether a message is a real question requiring a human response, then routes your answer back to the original sender.

## How it works

1. **Polls Signal** every 10 seconds via `signal-cli receive --json`
2. **AI classification** asks: "Is this a direct question requiring a human response?"
3. **Surfaces** to the SJBIS dashboard via `sjbis ask` CLI
4. **Waits** for you to answer on the dashboard
5. **Sends reply** back via `signal-cli send` using the sender's phone number
6. **Dedup cache** prevents the same message from surfacing twice

## Prerequisites

1. **Install signal-cli**:
   ```bash
   # macOS with Homebrew
   brew install signal-cli

   # Or download binary from:
   # https://github.com/AsamK/signal-cli/releases
   ```

2. **Register or link your Signal account**:
   ```bash
   # Option A: Register as primary device (requires phone with SMS)
   signal-cli -a +1234567890 register
   signal-cli -a +1234567890 verify CODE  # code sent via SMS

   # Option B: Link as secondary device (recommended — doesn't need phone number)
   signal-cli link --name sjbis-signal
   # Scan QR code with Signal app on your phone
   ```

3. **Verify it works**:
   ```bash
   signal-cli -a +1234567890 receive --json
   ```
   You should see recent messages as JSON.

4. **Set Fireworks API key** (for AI classification):
   ```bash
   export FIREWORKS_API_KEY="fw_..."
   ```

5. **Set up daemon URL** (if not on localhost):
   ```bash
   mkdir -p ~/.config/sjbis
   echo 'url = "http://dertog:7878"' > ~/.config/sjbis/daemon.toml
   ```

## Usage

### Run the plugin

```bash
cd plugins/signal
cargo run --release -- run
```

This polls Signal every 10 seconds for new messages, detects questions via AI, and surfaces them to your SJBIS dashboard.

### Test mode (dry run)

```bash
cargo run --release -- test --minutes 60
```

Shows what would be surfaced without actually creating notifications. Includes AI classification explanation for each message.

### Send a reply

```bash
cargo run --release -- send --to +14155551234 --text "Yes, that works for me"
```

## AI Classification

The plugin uses Fireworks AI (kimi-k2p6) to determine if a Signal message is a real question:

- **Real questions** → Surfaced to dashboard (e.g., "Want to grab lunch?", "Are you free?")
- **Greetings only** → Skipped (e.g., "Hey, how are you?")
- **Links/shares** → Skipped (e.g., "Check out this link")
- **Random messages** → Skipped
- **Group announcements** → Skipped

If the AI is unavailable, it falls back to regex heuristics.

## Answer Routing

When you answer on the dashboard:

- **Signal** → Plugin sends reply via `signal-cli send` to the sender's phone number
- **CLI** → `sjbis ask` returns answer via stdout to the calling process
- **HTTP** → `sjbis ask` blocks until answered, then returns answer

## Architecture

```
signal-cli daemon ──► sjbis-signal plugin ──► sjbis ask ──► SJBIS daemon
         │                    │                    │
         │  receive --json    │  AI classify       │  POST /ask
         │                    │  (Fireworks)       │
         │  send -m           │                    │  GET /wait/{id}
         │                    │                    │
         ▼                    ▼                    ▼
   Signal API            Dedup cache          Dashboard
   (send/receive)        (5-min window)       (web UI)
```

## Configuration

Edit the `Config::default()` in `src/main.rs` to customize:
- `signal_account`: Your Signal phone number (default: +1234567890)
- `poll_interval_secs`: How often to poll (default: 10s)
- `sjbis_binary`: Path to sjbis CLI (default: "sjbis")
- `agent_name`: Name shown in dashboard (default: "Signal")

## Troubleshooting

### "User is not registered"
You need to register or link signal-cli first. See Prerequisites above.

### No messages showing
- Check `signal-cli -a YOURNUMBER receive --json` manually
- The `--json` flag is required for machine-readable output
- Messages you've already read might not appear again (signal-cli marks them delivered)

### Replies not sending
- Verify the sender number is correct
- Check that signal-cli can send: `signal-cli -a YOURNUMBER send -m "test" YOURNUMBER`
- The plugin uses the sender's phone number (not name) for replies

### AI classifier fails
Check your Fireworks API key:
```bash
echo $FIREWORKS_API_KEY
```

## Future Improvements

- [ ] Use signal-cli JSON-RPC daemon mode for push notifications instead of polling
- [ ] Handle group chats (currently only handles direct messages)
- [ ] Support Signal voice notes / attachments
- [ ] Use SJBIS rules to filter Signal contacts (mute everything except whitelist)

## License

MIT
