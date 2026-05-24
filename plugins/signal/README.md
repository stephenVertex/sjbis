# SJBIS Signal Plugin

Surfaces Signal messages as SJBIS notifications using `signal-cli`.

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

## Usage

### Run the plugin

```bash
cd plugins/signal
cargo run -- run
```

This polls Signal every 10 seconds for new messages, detects questions, and surfaces them to your SJBIS dashboard.

### Test mode (dry run)

```bash
cargo run -- test --minutes 60
```

Shows what would be surfaced without actually creating notifications.

### Send a reply

```bash
cargo run -- send --to +14155551234 --text "Yes, that works for me"
```

## Architecture

```
signal-cli daemon ──► sjbis-signal plugin ──► SJBIS daemon
         │                                        │
         │  JSON-RPC / subprocess                 │  REST API
         │  (receive/send commands)               │  (ask/answer)
```

1. Plugin runs `signal-cli -a ACCOUNT receive --json` periodically
2. Parses JSON envelope output for text messages
3. Applies question heuristic (same as iMessage plugin)
4. Surfaces via `sjbis ask --blocking --agent-name Signal`
5. When you answer in dashboard, plugin sends reply via `signal-cli send`

## Configuration

Edit the `Config::default()` in `src/main.rs` to customize:
- `signal_account`: Your Signal phone number
- `poll_interval_secs`: How often to poll (default: 10s)
- `sjbis_binary`: Path to sjbis CLI

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
- The plugin currently uses sender name for replies — you may need to use the phone number instead

## Future Improvements

- [ ] Use signal-cli JSON-RPC daemon mode for push notifications instead of polling
- [ ] Cache sender name → phone number mapping for reliable replies
- [ ] Handle group chats (currently only handles direct messages)
- [ ] Support Signal voice notes / attachments
- [ ] Use SJBIS rules to filter Signal contacts (mute everything except whitelist)
