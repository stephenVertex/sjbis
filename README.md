# SJBIS — Stephen J Barr Information Surfacer

A human-in-the-loop notification dashboard. Agents (scripts, tools, AI systems) post questions via CLI or API. Humans answer them in a keyboard-centric web UI. Answers flow back to the caller synchronously or via webhooks.

![SJBIS in action](docs/images/sjbis-demo.gif)

SJBIS is a **universal information plane any tool can call**: one daemon, one CLI,
and a thin (optional) AI layer that turns arbitrary questions from arbitrary
callers into the right interaction for a human — surfaced, ranked, deduped, and
routed by rules you write in plain English.

> A fuller design narrative lives in [`SJBIS Architecture.html`](SJBIS%20Architecture.html).
> That document is the original v0.4 design vision annotated against what is
> actually built; this README describes the **as-built** system.

## Motivation

AI agents are increasingly doing real, autonomous work — but the highest-leverage
work still needs a human in the loop at the right moments: an approval, a
judgement call, a quick "which of these?". The hard part isn't running the agent;
it's surfacing the *one question that needs you* without drowning you in noise,
and routing your answer back to whatever process was waiting on it.

![Agents are taking on substantial, long-horizon work](docs/images/highlight-anthropic-claude.png)

SJBIS is built for that gap: let agents work autonomously, but give them a clean,
typed channel to ask a human when they should — and keep the human's attention
scarce and well-routed.

## The anatomy of a call

Any process emits a single `sjbis ask` command. The daemon receives it over HTTP,
optionally asks an LLM to fill in the gaps (urgency, the right renderer, dedupe),
pushes it to the dashboard, and waits. A typed answer comes back on the caller's
chosen channel — stdout, exit code, webhook, or a file — or a timeout, or a
"muted" signal if a rule dropped it.

Three things define every call:

- **The question** — required free text. The human (and optionally the AI router) reads it.
- **The answer shape** — one of `--yesno`, `--choices`, `--text`, `--number`, `--file`, `--diff`, `--ack`, `--pick`, `--schedule`. This picks the dashboard renderer. Skip it and pass `--guess-renderer` to let the AI choose.
- **The agent name** — `--agent-name "OpenCode"`, required. Stable across runs of the same caller; drives the card's source line, its glyph/color identity, and rule-matching. Optional `--instance` appends per-session detail (e.g. `"Session s7b3d11"`).

Plus a **reply channel** (`--reply-to`, default `stdout` when `--blocking`) and an
optional **deadline** (`--deadline 6m`). See [Working agreement for
agents](#working-agreement-for-agents) for the blocking + timeout pattern.

## Topology

SJBIS is a **client/server split**, even though it's one binary:

- **Daemon** — runs on an always-on host and owns everything: PostgreSQL, the dashboard, SSE, and the HTTP API. Typically a systemd user service.
- **Client** — the same `sjbis` binary run from your workstation (or any agent), pointed at the daemon's URL. It does not run a local daemon or database.

The two talk over plain HTTP/JSON. Point the client at the daemon by setting a
URL in `~/.config/sjbis/daemon.toml` (or the `SJBIS_DAEMON` env var); otherwise
it defaults to `http://localhost:7878` — handy if you run the daemon locally.

```toml
# ~/.config/sjbis/daemon.toml  (on the client)
url = "http://your-daemon-host:7878"
```

> **The author's reference deployment.** Stephen runs the daemon on a home-LAN
> host (`dertog`, reachable at `http://192.168.0.138:7878`) as a systemd user
> service, backed by a separate PostgreSQL server, and uses the `sjbis` CLI on a
> Mac as a client. Concrete commands below use those names/addresses as worked
> examples — substitute your own host and IP.

## Quick Start (Client)

```bash
# 1. Build / install the CLI locally
cargo install --path .

# 2. Point the client at your daemon (use your daemon host's URL)
mkdir -p ~/.config/sjbis
cat > ~/.config/sjbis/daemon.toml << 'EOF'
url = "http://your-daemon-host:7878"
EOF

# 3. Open the dashboard (served by the daemon)
open http://your-daemon-host:7878

# 4. Post a question from any agent (goes to the daemon)
sjbis ask --question "Deploy to prod?" --yesno \
  --agent-name deploybot --blocking
```

> If you haven't stood up a daemon yet, see [Server Deployment](#server-deployment).
> You only need that section to set up or update the daemon host, not for
> day-to-day client use.

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
# Set your daemon host (the reference deployment uses `dertog`)
HOST=your-daemon-host

# On the server
ssh "$HOST" 'mkdir -p ~/sjbis ~/.config/sjbis'

# Copy binary
scp target/x86_64-unknown-linux-musl/release/sjbis "$HOST":~/sjbis/

# Copy static files (dashboard UI). scp avoids needing rsync on the host.
scp static/*.jsx static/*.css static/*.html "$HOST":~/sjbis/static/

# Configure the database on the daemon host (point dsn at your Postgres)
ssh "$HOST" 'cat > ~/.config/sjbis/database.toml' << 'EOF'
[database]
dsn = "postgresql://user:pass@your-postgres-host:5432/sjbis"
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
# On the daemon host
curl http://localhost:7878/health   # → "ok"
curl http://localhost:7878/list       # → [] (empty initially)

# From a client, hit the daemon by its URL
curl http://your-daemon-host:7878/health   # → "ok"
```

> In practice, build + deploy + restart + status checks are automated by
> [`build-and-deploy.sh`](build-and-deploy.sh), which scp's the binary and
> static assets to the daemon host and restarts the service. The manual steps
> above document what that script does under the hood. The host and URL are
> configurable via `SJBIS_REMOTE_HOST` / `SJBIS_REMOTE_URL` env vars (defaults
> target the reference `dertog` deployment).

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

One process boundary: the **daemon**. It exposes an HTTP/JSON API (built on
`axum`), persists everything to PostgreSQL, and pushes live updates to the
dashboard over Server-Sent Events. The LLM is optional and used in narrow,
replaceable roles.

### Components

| Component | Role |
|---|---|
| **Ingress** | The HTTP/JSON API the CLI calls (`/ask`, `/answer/{id}`, `/list`, …). Validates the answer shape, applies `--id` idempotency, writes to the store. No LLM here — speed matters. |
| **AI Router** *(optional)* | Reads the question + caller, and fills gaps: suggests a renderer (when missing or `--guess-renderer`), predicts urgency to break ties, and flags likely duplicates. Bypassed entirely when `--privacy private` or when no API key is set. |
| **Rule Engine** | Each rule is a plain-English line plus a compiled JSON filter. Compilation is a one-time LLM call (or a fast offline pattern matcher); matching is deterministic. Rules can mute, snooze, re-prioritize, or auto-answer. Evaluated in priority order; time-bounded rules auto-expire. |
| **Queue + Store** | PostgreSQL over `sqlx`. Tables: `notifications` (in-flight + history), `rules`, `agents`. Migrations live in the binary and auto-run on start. The daemon is stateless beyond this. |
| **Identity** | Maps `--agent-name` to a stable glyph + color via deterministic hashing (no LLM). `--instance` is shown on the card but does not affect identity or rules. |
| **Response Router** | When the human answers, formats and delivers the result per `--reply-to`: stdout, webhook (with retry), file (atomic write), or exit code. Records the full answer trail (renderer, latency, via). |
| **Dashboard** | The browser app served from `static/`. Subscribes over SSE for live updates and POSTs answers back. Renders the renderer the caller (or router) chose. |
| **LLM Provider** *(optional)* | Any OpenAI-compatible endpoint. Default: Fireworks, model `accounts/fireworks/models/kimi-k2p6`. Enabled only when `FIREWORKS_API_KEY` is set; the daemon stays fully deterministic without it. |

### Lifecycle of one ask

A blocking yes/no, end to end:

1. **Caller** emits `sjbis ask … --blocking --deadline 6m`; the CLI POSTs JSON to the daemon.
2. **Ingress** validates the shape, checks `--id` for idempotency, creates a `notifications` row.
3. **AI Router** *(if enabled)* makes one short LLM call to classify, predict urgency, and detect duplicates.
4. **Rule Engine** applies your rules in priority order — pass through, re-prioritize, mute, snooze, or auto-answer.
5. **Dashboard** receives an SSE event; the card appears (urgency drives how loudly).
6. **Human** answers (click, type, drag, or keyboard shortcut).
7. **Response Router** delivers the answer back over the caller's channel.
8. **Store** persists the full answer trail (available via `/history` and the dashboard's recent rail).

Steady state, an ask makes **at most one** LLM call (often zero — the router is
optional and cache-friendly).

### The AI layer (scoped tight)

The AI is deliberately small and explainable. It runs in two narrow roles today:

- **Routing & ranking** — one prompt per ask: suggest a renderer, predict urgency, flag dedupe candidates. The caller's explicit flags win; the model never rewrites your question. On provider outage it falls back to the caller's claims.
- **Rule compilation** — one prompt when you *write* a rule, turning English into a JSON filter that the daemon then evaluates deterministically.

Both are gated on `FIREWORKS_API_KEY`; with no key, SJBIS is a fully
deterministic notification surfacer.

> **Not yet built (planned in the design doc):** authentication / bearer tokens,
> Tailscale transport + device identity, agent token issuance from `sjbis
> register`, persistent gRPC streams, language SDKs, and `redact-pii` masking.
> Today the daemon trusts any client that can reach it on the LAN — **keep it off
> the public internet.**

**Key design decisions:**
- **Blocking ask is opt-in** (`--blocking`). Default is fire-and-forget so automated workflows never hang.
- **Human input never blocks by default** — agents must explicitly request synchronous answers.
- **Keyboard-centric** — J/K navigate, Enter open, 1–9 answer, S snooze, D dismiss, T tweaks.
- **Universal snooze** with deadline cap (cannot snooze past auto-approve deadline).
- **Optional human note** attached to every answer, returned to the caller.
- **AI is optional and bounded** — at most one LLM call per ask, and the caller's explicit input always wins.

## CLI Commands

The `sjbis` binary is both the client (talks to the daemon) and the daemon itself.

| Command | What it does |
|---|---|
| `sjbis ask …` | Post a question. Returns an id immediately; blocks for an answer with `--blocking`. |
| `sjbis answer <id> --answer <v>` | Record an answer on behalf of the caller (e.g. an agent's auto-pick after a timeout). Supports `--via` and `--note`. |
| `sjbis wait <id>` | Reattach to a posted question and block until it resolves. |
| `sjbis status <id>` | Print a notification's state (open / answered / cancelled / timed_out / dismissed). |
| `sjbis list [--json]` | List open notifications. |
| `sjbis cancel <id>` | Withdraw an unanswered question. |
| `sjbis dismiss <id>` | Mark as seen without answering; no reply sent. |
| `sjbis rule add\|allow\|list\|rm` | Manage filtering rules (see [API](#post-rules--create-filtering-rules)). |
| `sjbis entity add\|list\|show\|rm` | Manage named contact groups used in rules. |
| `sjbis register --agent-name <n>` | Register an agent identity (name + optional glyph/color). |
| `sjbis prime` | Print the agent primer (working agreement, question types, daemon status). |
| `sjbis upgrade` | Self-update from GitHub Releases (see [Upgrading](#upgrading)). |
| `sjbis daemon start\|stop\|status` | Daemon lifecycle. `start --port 7878 [--background]`. |

Run `sjbis prime` first when wiring up a new agent — it prints the live daemon
status and the exact pattern to follow.

## Working agreement for agents

When an agent (script or AI) needs a human decision, the intended pattern is a
**blocking ask with a deadline**, then **proceed on timeout**:

```bash
# 1. Ask, blocking, with a deadline. --json gives a structured result.
res=$(sjbis ask --question "Approve PR #412?" --yesno \
        --agent-name codebot --deadline 1m --blocking --json)

# 2. via == "timed_out" means the deadline passed with no human answer.
via=$(echo "$res" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("via",""))')

# 3. If it timed out, DON'T hang — apply best judgement, then INFORM the
#    server so the dashboard shows the auto-pick:
if [ "$via" = "timed_out" ]; then
  id=$(echo "$res" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
  sjbis answer "$id" --answer "no" --via caller-timeout \
    --note "No reply in time — held off because the PR touches auth."
fi
```

The daemon caps a blocking wait at the deadline and returns promptly, so agents
never build their own poll loops. Silence means "use your judgement," not a hang.
Always check the `note` field on a real answer — humans can attach follow-up
context there.

## API for Agents

> The examples below use `http://localhost:7878` for brevity. From a **client**,
> substitute your daemon's URL (the reference deployment uses
> `http://192.168.0.138:7878`), or set `SJBIS_DAEMON` /
> `~/.config/sjbis/daemon.toml` so the `sjbis` CLI uses it automatically.

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

### `~/.config/sjbis/daemon.toml` (client)

Points the `sjbis` CLI at the daemon. Set this to your daemon host's URL:

```toml
url = "http://your-daemon-host:7878"
```

### `~/.config/sjbis/database.toml` (daemon host)

```toml
[database]
dsn = "postgresql://user:pass@your-postgres-host:5432/sjbis"
```

### `~/.config/sjbis/entities.toml`

Named contact lists that expand in rules:

```toml
[groups]
family = ["Alice", "Bob", "Mom", "Dad"]
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

## Upgrading

`sjbis` can update itself in place from GitHub Releases — no need to rebuild or
re-run the deploy script for a routine version bump.

```bash
sjbis upgrade --check          # see if a newer release exists (no download)
sjbis upgrade                  # download the latest release and replace this binary
sjbis upgrade --tag v0.1.3     # install a specific tagged release
sjbis upgrade --force          # reinstall even if already on the latest version
```

How it works:

- Queries the GitHub Releases API for `stephenVertex/sjbis`, compares the running
  `CARGO_PKG_VERSION` against the latest tag (build metadata after `+` is ignored).
- Downloads the asset matching this platform's Rust target triple
  (`sjbis-<triple>.tar.gz`), extracts the `sjbis` binary, and atomically swaps it
  in via [`self-replace`](https://crates.io/crates/self-replace).
- `--check` works on any platform; a real install requires a published asset for
  your platform. Supported targets: **macOS Apple Silicon**
  (`aarch64-apple-darwin`) and **Linux x86_64** (`x86_64-unknown-linux-musl`).
  Other platforms get a clear "no prebuilt release" message.

After upgrading the daemon host, restart the service so the new binary takes
effect:

```bash
systemctl --user restart sjbis
```

Or use the helper script, which runs `sjbis upgrade` on the daemon host,
refreshes the dashboard assets, restarts the service, and runs health checks:

```bash
./update-dertog.sh            # update the daemon host to the latest release
./update-dertog.sh v0.1.3     # update to a specific tag
```

`update-dertog.sh` is the release-driven counterpart to `build-and-deploy.sh`:
use `update-dertog.sh` to pull a published release, and `build-and-deploy.sh`
when you want to push your local working tree straight to the host without
cutting a release. The target host is configurable via the same
`SJBIS_REMOTE_*` env vars (defaults target the reference `dertog` deployment).
The remote must already have a `sjbis` new enough to have the `upgrade`
subcommand (≥ 0.1.2); for a first install on an older box, run
`build-and-deploy.sh` once.

### Release builds (CI)

Release assets are produced by `.github/workflows/release.yml`, triggered on a
`v*` tag push (or manual `workflow_dispatch`). It builds the macOS Apple Silicon
and Linux musl binaries on self-hosted runners (with GitHub-hosted fallback),
tarballs them with a `.sha256`, and attaches them to the GitHub Release that
`sjbis upgrade` reads from.

```bash
# Cut a release
git tag v0.1.3 && git push origin v0.1.3
```

## Environment Variables

| Variable | Purpose |
|---|---|
| `SJBIS_DAEMON` | Daemon URL the CLI talks to. Defaults to `http://localhost:7878`. Set this (or `~/.config/sjbis/daemon.toml`) to point at a remote daemon host. |
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
