# SJBIS — Stephen J Barr Information Surfacer

A human-in-the-loop notification dashboard. Agents (scripts, tools, AI systems) post questions via CLI or API. Humans answer them in a keyboard-centric web UI. Answers flow back to the caller synchronously or via webhooks.

## Quick Start (Local)

```bash
# 1. Build
cargo build --release

# 2. Configure PostgreSQL
cat > ~/.config/sjbis/database.toml << 'EOF'
[database]
dsn = "postgresql://user:pass@localhost:5432/sjbis"
EOF

# 3. Start daemon
./target/release/sjbis daemon start --port 7878

# 4. Open dashboard
open http://localhost:7878

# 5. Post a question from any agent
./target/release/sjbis ask --question "Deploy to prod?" --yesno \
  --agent-name deploybot --blocking
```

## Server Deployment

SJBIS deploys as a single static binary + PostgreSQL. No runtime dependencies.

### 1. Build (macOS → Linux x86_64)

Requires [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild):

```bash
# Install zig and cargo-zigbuild (one-time)
brew install zig
cargo install cargo-zigbuild

# Build fully static musl binary
cargo zigbuild --target x86_64-unknown-linux-musl --release

# Binary: target/x86_64-unknown-linux-musl/release/sjbis
# Size: ~7.8MB, statically linked, runs on any Linux x86_64
```

### 2. Server Setup

```bash
# On the server (e.g. dertog)
mkdir -p ~/sjbis ~/.config/sjbis

# Copy binary
scp target/x86_64-unknown-linux-musl/release/sjbis dertog:~/sjbis/

# Copy static files (dashboard UI)
rsync -av static/ dertog:~/sjbis/static/

# Configure database
cat > ~/.config/sjbis/database.toml << 'EOF'
[database]
dsn = "postgresql://user:pass@host:5432/sjbis"
EOF
```

### 3. systemd User Service

```bash
# Create service file
cat > ~/.config/systemd/user/sjbis.service << 'EOF'
[Unit]
Description=SJBIS — Stephen J Barr Information Surfacer
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=%h/sjbis
ExecStart=%h/sjbis/sjbis daemon start --port 7878
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
EOF

# Enable and start
systemctl --user daemon-reload
systemctl --user enable sjbis
systemctl --user start sjbis

# Management
systemctl --user status sjbis     # check health
systemctl --user restart sjbis      # restart
systemctl --user stop sjbis         # stop
journalctl --user -u sjbis -f      # view logs
```

The service auto-starts on user login and auto-restarts on crash.

### 4. Verify

```bash
curl http://localhost:7878/health   # → "ok"
curl http://localhost:7878/list       # → [] (empty initially)
```

## Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Agent     │────▶│  SJBIS API  │────▶│  PostgreSQL │
│  (script)   │     │   :7878     │     │  (state)    │
└─────────────┘     └──────┬──────┘     └─────────────┘
                           │
                    ┌──────┴──────┐
                    │   Dashboard   │
                    │  (browser)    │
                    │  SSE + REST   │
                    └─────────────┘
```

**Key design decisions:**
- **Blocking ask is opt-in** (`--blocking`). Default is fire-and-forget so automated workflows never hang.
- **Human input never blocks by default** — agents must explicitly request synchronous answers.
- **Keyboard-centric** — J/K navigate, Enter open, 1–9 answer, S snooze, D dismiss, T tweaks.
- **Universal snooze** with deadline cap (cannot snooze past auto-approve deadline).
- **Optional human note** attached to every answer, returned to the caller.

## API for Agents

### POST /ask — create a notification

```bash
curl -X POST http://localhost:7878/ask \
  -H "Content-Type: application/json" \
  -d '{
    "question": "Deploy to prod?",
    "agent_name": "deploybot",
    "question_type": "yesno",
    "urgency": 4,
    "blocking": true
  }'
```

### GET /wait/{id} — block until answered

For `--blocking` callers. Returns immediately if already answered/dismissed/cancelled/timed_out.

### GET /list — open notifications

### GET /state — full dashboard state (notifications + history + rules + agents)

### POST /answer/{id} — submit answer

```bash
curl -X POST http://localhost:7878/answer/sjbis-AbCdEfGh \
  -H "Content-Type: application/json" \
  -d '{"answer": "Yes", "via": "dashboard", "note": "Security scan passed"}'
```

### POST /dismiss/{id} — mark as seen without answering

No reply sent to the caller. Wakes up blocking waiters with empty answer.

### POST /snooze/{id} — push back by N minutes

Capped at the notification's deadline if one exists.

### POST /rules — create filtering rules

Natural language — no syntax to memorize:

```bash
curl -X POST http://localhost:7878/rules \
  -H "Content-Type: application/json" \
  -d '{"text": "mute all iMessage except family for 1h"}'
```

This creates a mute-all + surface-exceptions ruleset automatically.

## Configuration Files

### `~/.config/sjbis/database.toml`

```toml
[database]
dsn = "postgresql://user:pass@host:5432/sjbis"
```

### `~/.config/sjbis/entities.toml`

Named contact lists that expand in rules:

```toml
[groups]
family = ["Jeff", "Carmen", "Mom", "Dad"]
work   = ["boss@company.com", "team-lead"]
```

## Plugins

### iMessage Plugin

Surfaces iMessage texts as SJBIS notifications. Requires macOS + Full Disk Access.

```bash
cd plugins/imessage
cargo run -- run          # daemon mode
cargo run -- test         # dry run
cargo run -- send ...     # send reply back
```

### Signal Plugin

Surfaces Signal messages. Requires `signal-cli` linked to your account.

```bash
cd plugins/signal
cargo run -- run          # daemon mode
cargo run -- test         # dry run
cargo run -- send ...     # send reply back
```

## Question Types

| Type | Flag | How human answers |
|---|---|---|
| Yes/No | `--yesno` | Y/N keys or buttons |
| Multi-choice | `--choices a,b,c` | 1–9 keys |
| Free text | `--text` | Type + Enter |
| Numeric | `--number` | Slider or type |
| File upload | `--file` | Drag & drop |
| Diff approval | `--diff` | Approve / Reject |
| Acknowledge | `--ack` | Any key to dismiss |
| Pick list | `--pick items.json` | 1–9 keys |
| Schedule | `--schedule slots.json` | 1–9 keys |

## Database Migrations

Migrations live in `migrations/` and auto-run on daemon startup via `sqlx::migrate!`.

- `001_initial.sql` — base schema (notifications, rules, agents)
- `002_add_snooze.sql` — `snooze_until` column
- `003_add_note.sql` — `note TEXT` column
- `004_add_detail_markdown.sql` — `detail_markdown` for rich text
- `005_add_rule_priority.sql` — `priority` for rule evaluation order

## Environment Variables

| Variable | Purpose |
|---|---|
| `SJBIS_DAEMON` | Override daemon URL (default: `http://localhost:7878`) |
| `FIREWORKS_API_KEY` | Enable AI-powered rule compilation and renderer guessing |

## Troubleshooting

### "cached plan must not change result type"

The daemon caches prepared statements. After adding a migration column, restart:
```bash
systemctl --user restart sjbis
```

### iMessage plugin: "User is not registered"

Grant Full Disk Access to your terminal app (not the `.app` bundle), then:
```bash
./sjbis-imessage run
```

### Port already in use

```bash
lsof -i :7878
systemctl --user restart sjbis
```

## License

MIT
