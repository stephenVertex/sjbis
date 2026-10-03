# SJBIS — Stephen J Barr Information Surfacer

A human-in-the-loop notification dashboard. Agents (scripts, tools, AI systems) post questions via CLI or API. Humans answer them in a keyboard-centric web UI. Answers flow back to the caller synchronously or via webhooks.

![SJBIS in action](https://post-genius-media.s3.amazonaws.com/shup/project/sjbis/sjbis-demo.gif)

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

![Agents are taking on substantial, long-horizon work](https://post-genius-media.s3.amazonaws.com/shup/project/sjbis/highlight-anthropic-claude.png)

> Excerpt from Anthropic's [_Recursive self-improvement_](https://www.anthropic.com/institute/recursive-self-improvement).

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
Use the daemon's complete base URL, including a path prefix such as `/sjbis`
when one is configured.

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
scp static/*.js static/*.jsx static/*.css static/*.html "$HOST":~/sjbis/static/

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

## Stable card links and path prefixes

### Canonical card URLs

Every notification has a stable browser URL of the form
`{dashboard-base}/card/{id}` for mail, chat, agent, bookmark, and copy-link
consumers. For example:

```text
http://your-daemon-host:7878/card/sjbis-AbCdEfGh
https://dertog.tailb4b58.ts.net/sjbis/card/sjbis-AbCdEfGh
```

Opening a card from the list pushes its canonical URL into browser history; the
Back and Forward buttons move between the list and card views. Reloading the
URL opens the same card directly. The Focus view's copy-link control copies the
absolute canonical URL, including the dashboard's configured path prefix.

The older `{dashboard-base}/?q_id={id}` form remains supported as an alias. Once
an existing card loads, the browser replaces the alias with its canonical
`/card/{id}` URL. Direct links work for open cards and for every terminal
status: `answered`, `cancelled`, `muted`, `timed_out` (deadline-expired), and
`dismissed`. Terminal cards open read-only. An unknown ID shows a clear
**Card not found** state rather than silently returning to the list.

Card URLs are a browser routing feature, not a producer schema change. In
particular, `Notification` responses do **not** gain a `card_url` field:

- Browser sharing combines `window.location.origin`, the dashboard base path,
  and the notification `id`.
- Mail, chat, and agent integrations combine their configured public dashboard
  base (for example `https://dertog.tailb4b58.ts.net/sjbis`) with
  `/card/{id}`. The existing notification `id` returned by `POST /ask` is all
  they need.
- Existing web list consumers keep reading `notifications`, `history`, `rules`,
  `agents`, and `version` from `GET /state`. Card rendering continues to use
  existing notification fields for identity and source, question and type,
  urgency, choices, deadline, status, and timestamps.
- SSE payloads are unchanged: events retain their event-specific
  `notification`, `envelope`, `rule`, or `id` fields.
- `sjbis status` still consumes `id`, `status`, `agent_name`, `question`,
  `answer`, `note`, and `answered_at`. The native iOS client still decodes the
  existing full `SjbisNotification` and `DashboardState` models. Neither client
  needs a schema migration.

The original [`SJBIS Architecture.html`](SJBIS%20Architecture.html) reference
and the native UI under `ios/` are intentionally outside this web-routing
change. These browser URLs do not add iOS Universal Links, alter the producer
wire format, or configure a reverse proxy.

### Serving the dashboard below `/sjbis`

Start the daemon with an explicit base path when a reverse proxy exposes SJBIS
below a prefix:

```bash
sjbis daemon start --port 7878 --base-path /sjbis
```

This mounts the dashboard, static assets, REST API, SSE stream, and health check
below the same prefix; for example, the health endpoint becomes
`http://127.0.0.1:7878/sjbis/health`. The default base path remains `/`.

The reverse proxy must preserve the prefix all the way to the application. In
other words, an external request for `/sjbis/card/{id}` must reach the daemon as
`/sjbis/card/{id}`, not `/card/{id}`. Configure CLI and native clients with the
complete prefixed daemon URL as well:

```toml
# ~/.config/sjbis/daemon.toml
url = "https://dertog.tailb4b58.ts.net/sjbis"
```

```bash
export SJBIS_DAEMON=https://dertog.tailb4b58.ts.net/sjbis
sjbis list
```

Tailscale Serve configuration is deliberately deferred. The exact follow-up
for the reference tailnet host is:

1. Change the service command to
   `sjbis daemon start --port 7878 --base-path /sjbis` and restart it.
2. On that host, confirm the installed Tailscale version supports these flags,
   then run:

   ```bash
   tailscale serve --bg --https=443 --set-path=/sjbis http://127.0.0.1:7878/sjbis
   ```

3. Verify `https://dertog.tailb4b58.ts.net/sjbis/health`, then open and reload
   `https://dertog.tailb4b58.ts.net/sjbis/card/{existing-id}`.

No Tailscale Serve state or other deployment infrastructure is changed by this
repository update.

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
| `sjbis status <id>` | Print a notification's state (open / answered / cancelled / muted / timed_out / dismissed). |
| `sjbis list [--json]` | List open notifications. |
| `sjbis cancel <id>` | Withdraw an unanswered question. |
| `sjbis dismiss <id>` | Mark as seen without answering; no reply sent. |
| `sjbis rule add\|allow\|list\|rm` | Manage filtering rules (see [API](#post-rules--create-filtering-rules)). |
| `sjbis entity add\|list\|show\|rm` | Manage named contact groups used in rules. |
| `sjbis triage create\|list\|show` | Create and inspect durable batch-review queues. |
| `sjbis triage decide\|clear` | Append or clear an item's current verdict. |
| `sjbis triage refresh\|attach` | Reconcile producer files or resolve an ambiguous move explicitly. |
| `sjbis triage close\|reopen\|export` | Freeze/unfreeze mutations and consume revision events. |
| `sjbis register --agent-name <n>` | Register an agent identity (name + optional glyph/color). |
| `sjbis prime` | Print the agent primer (working agreement, question types, daemon status). |
| `sjbis upgrade` | Self-update from GitHub Releases (see [Upgrading](#upgrading)). |
| `sjbis daemon start\|stop\|status` | Daemon lifecycle. `start --port 7878 [--base-path /sjbis] [--background]`. |

Run `sjbis prime` first when wiring up a new agent — it prints the live daemon
status and the exact pattern to follow.

### Batch triage queues

Triage queues turn a directory or JSON list of independent Markdown items into
a durable review session. Each item receives one of five wire verdicts:
`schedule`, `delete`, `needs_replan`, `merge_into`, or `leave_captured`. Only
`merge_into` carries a target, and that target is another declared item id in
the same queue.

Create from one or more root-relative globs, or from a JSON list:

```bash
sjbis triage create planning-review --root ./notes \
  --glob '**/*_analysis.md' --glob 'followups/*.md'

sjbis triage create planning-review --root ./notes \
  --json-list ./triage-items.json
```

The JSON list is an array of objects. Each entry is exactly one of:

```json
[
  {"path": "reviews/alpha_analysis.md"},
  {
    "id": "inline-question",
    "markdown": "# Question\n\nChoose a disposition.",
    "source": {"note_id": "ys-example", "revision": 4}
  }
]
```

Path-backed ids come from `item:` in YAML-style frontmatter when present;
otherwise they come from the basename after removing `--strip-suffix`
(default `_analysis.md`). Id collisions reject the whole create and name every
colliding path. Paths are canonicalized beneath `--root`; absolute paths,
parent traversal, and symlinks that escape the root are rejected. Supported
glob syntax is `*`, `?`, character classes such as `[a-z]`, and recursive
`**`; brace expansion is not supported.

Catalog order is deterministic: path-backed items first in lexical canonical
relative-path order, then inline items in lexical id order. JSON array order
and filesystem enumeration order are not retained.

```bash
sjbis triage list
sjbis triage --json show planning-review

sjbis triage decide planning-review alpha schedule
sjbis triage decide planning-review duplicate-note merge_into --target canonical-note
sjbis triage clear planning-review alpha

sjbis triage refresh planning-review
sjbis triage attach planning-review moved-note archive/moved_analysis.md

sjbis triage close planning-review
sjbis triage export planning-review --since '' --limit 100
sjbis triage reopen planning-review
```

Decisions are append-only revisions. `clear` appends an explicit null verdict;
it does not delete history. Progress counts each item's latest non-null verdict
once. A non-empty queue with a latest verdict for every item reports
`complete: true`, but remains `status: "open"` until explicitly closed.

#### Incremental export and cursors

Every export is a self-contained envelope with `queue`, the complete `catalog`,
revision `items`, and an opaque `nextCursor`. Omitted or empty `since` means
stream origin. Persist `nextCursor` only after processing the returned events,
then pass it unchanged in the next consumer session:

```bash
sjbis triage --json export planning-review --since '' --limit 100 > page.json
jq -r '.nextCursor' page.json > planning-review.cursor

# Later, potentially from another process or host:
cursor=$(cat planning-review.cursor)
curl --fail-with-body \
  "$SJBIS_DAEMON/triage/queues/planning-review/export?since=$cursor&limit=100" \
  > next-page.json
```

Do not derive a cursor from `decided_at`: event ids are the durable ordering
key, and timestamps can collide. Consumers should deduplicate or audit with
`event_id`, not counts alone. Queue creation does not return a cursor.

#### Snapshots and refresh

Path-backed Markdown, SHA-256, and capture time form the served snapshot; the
captured source path is also visible and may advance when a move is resolved.
`triage refresh` rereads the queue's persisted glob or JSON-list source
specification, but changed producer text never overwrites the stored path
snapshot. Instead, the item becomes `content_changed` and continues to serve
the original Markdown and hash on which earlier verdicts were based.

Refresh freshness states are:

| State | Meaning |
|---|---|
| `current` | The declared id and hash still match, or one unambiguous hash move reattached the original id. |
| `missing` | The original item was not observed and its hash was not found elsewhere. |
| `content_changed` | The declared id remains, but producer bytes differ from the captured snapshot. |
| `ambiguous` | Hash correspondence has multiple possible missing items or observed paths; candidate paths are returned. |

Hash reattachment occurs only for exactly one missing path item and exactly one
new observed path with the same bytes. Ambiguity does not fail the whole
refresh. Resolve it with `triage attach QUEUE ITEM PATH`, which requires a file
inside the queue root whose hash matches the stored snapshot, or remove the
duplicate candidates and refresh again. Unknown ids and hashes become new
items. Inline items are refreshed by re-importing the same id; their Markdown,
hash, provenance, and capture time are replaced together. If local discovery
fails, the CLI stops before posting a refresh, so an unreadable producer cannot
be mistaken for an empty source.

#### Merge direction and queue freezing

A `merge_into` revision is one directed edge from the decided item to its
target: `duplicate -> canonical`. The API validates the target against the
same queue at decision time and rejects self-targets and direct reciprocal
edges. Consumers read only that direct edge. SJBIS does not compute transitive
closure, rewrite chains to a canonical root, or infer targets from comments.

Closing a queue is a mutation freeze, not deletion. While closed, decision
patches (including clear), refresh, and attach return HTTP `409` with code
`queue_closed`; their CLI forms exit `3` with `queue is closed; reopen first`.
Show, list, and export remain available. `reopen` lifts the freeze, including
for an incompletely decided queue.

#### HTTP contract

The CLI discovers and hashes local files before calling the daemon. Direct HTTP
producers must send the already captured observations themselves.

| Method and path | Contract |
|---|---|
| `POST /triage/queues` | Create from `{name, root, strip_suffix, source_spec, items}`; returns `201`. |
| `GET /triage/queues` | List queue summaries. |
| `GET /triage/queues/{queue}` | Return `{queue, catalog}` by id or unique name. |
| `PATCH /triage/queues/{queue}/items/{item}/decision` | Append a revision; use `{"verdict": null}` to clear. |
| `POST /triage/queues/{queue}/refresh` | Reconcile `{"items": [...]}` captured from the persisted source. |
| `POST /triage/queues/{queue}/items/{item}/attach` | Attach `{"path": "...", "markdown": "..."}` to a matching stale snapshot. |
| `POST /triage/queues/{queue}/close` | Freeze mutating item operations. |
| `POST /triage/queues/{queue}/reopen` | Return the queue to open status. |
| `GET /triage/queues/{queue}/export?since=&limit=100` | Return the catalog plus ordered revision events and `nextCursor`. |

An absent `verdict` field means "retain the previous verdict"; explicit JSON
null means clear; an empty string is invalid. `merge_into` requires `target`,
and other verdicts reject it. Export limits are 1 through 1000.

Expected failures are explicit: duplicate queue names return `409` with the
existing queue id and resume command; closed mutations return `409`; missing
queues/items return `404`; invalid payloads, targets, cursors, limits, paths,
source shapes, and identity collisions return `400` at the HTTP boundary (or
exit `1` when rejected locally/by the CLI). Duplicate queue creation never
silently resumes. Ambiguous refresh is a successful result with counts and
candidate paths, not an error.

#### Deliberate v1 exclusions

Triage v1 has no persistent file watcher, deadlines, automatic/default
verdicts, or timeout-generated revisions. It does not produce Yesod
`open_questions` or write answers back to planning results. It has no
spotlight-md dependency, per-item highlight threads, brace expansion,
user-defined/JSON-array ordering, transitive merge traversal, or canonical
merge-root rewriting. Inputs are filesystem globs or the documented JSON list,
and the interchange format is JSON; richer producer integrations remain
separate work.

The executable acceptance scenario is `tests/triage-e2e.sh`. It creates a
temporary PostgreSQL cluster and daemon, copies real fixtures, drives separate
CLI and HTTP cursor sessions, and removes all temporary state on exit. Set
`SJBIS_BIN` to reuse an existing binary or `SJBIS_TRIAGE_E2E_KEEP=1` to retain
failure artifacts.

### Supplying ask content over stdin

Interactive calls can continue to pass content with `--question`, `--detail`,
and `--detail-markdown`, as the examples above do. For an integration that
launches `sjbis ask` as a subprocess and needs its message content kept out of
that child process's argument list, use `--content-stdin` instead:

```bash
# The JSON below is stdin for sjbis, not an argument to either command.
cat <<'JSON' | sjbis ask --content-stdin --yesno \
  --agent-name mail-triage --blocking --json
{
  "question": "<question text>",
  "detail": "<optional plain-text context>",
  "detail_markdown": "<optional markdown context>"
}
JSON
```

`--content-stdin` reads one JSON object. Its `question` string is required;
`detail` and `detail_markdown` are optional strings, and unknown content fields
are rejected. It cannot be combined with `--question`, `--detail`, or
`--detail-markdown`; those flags remain the existing argv-content mode. All
other `ask` flags, including `--agent-name`, answer-shape flags, `--blocking`,
and `--json`, work unchanged.

This is intended for subprocess integrations that carry message bodies, links,
or tokens: write that payload to stdin rather than constructing a command line
with it. The mode avoids ordinary inspection of the `sjbis ask` process's
arguments; it is not encryption or a broader claim about content after `sjbis`
has read it and sent it to the daemon.

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
> `~/.config/sjbis/daemon.toml` so the `sjbis` CLI uses it automatically. Include
> the configured base path, for example `https://host.example/sjbis`.

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
# With `sjbis daemon start --base-path /sjbis`:
# url = "https://your-daemon-host/sjbis"
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
| `SJBIS_DAEMON` | Complete daemon base URL the CLI talks to, including any configured path prefix. Defaults to `http://localhost:7878`. Set this (or `~/.config/sjbis/daemon.toml`) to point at a remote daemon host. |
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
