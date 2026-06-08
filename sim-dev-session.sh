#!/usr/bin/env bash
#
# sim-dev-session.sh — a visual demo for SJBIS screen capture.
#
# Simulates TWO concurrent source agents designing different web apps, deep in
# implementation details, plus an inbound email. Questions are path-dependent
# (each answer changes the next question) and use a mix of renderers and
# urgencies so the dashboard shows a rich, live stream of cards.
#
# Each question blocks with a deadline; answer some on the dashboard for the
# camera, let others auto-pick on timeout (recorded via --via caller-timeout).
#
# Usage:
#   ./sim-dev-session.sh                 # default 25s deadline
#   SJBIS_SIM_DEADLINE=15s ./sim-dev-session.sh
#
set -uo pipefail

DEADLINE="${SJBIS_SIM_DEADLINE:-25s}"

# ── logging (all to stderr so they never pollute captured stdout values) ────
bold() { printf '\033[1m%b\033[0m\n' "$*" >&2; }
step() { printf '\n\033[1;36m▶ [%s] %s\033[0m\n' "$1" "$2" >&2; }
human(){ printf '  \033[32m← human:\033[0m %s\n' "$*" >&2; }
auto() { printf '  \033[33m⏱ timeout → auto-pick:\033[0m %s\n' "$*" >&2; }
note() { printf '  \033[2m%s\033[0m\n' "$*" >&2; }

# ── ask helper ──────────────────────────────────────────────────────────────
# ask <agent> <instance> <question> <flag-and-args...>  -> raw JSON on stdout
ask() {
  local agent="$1" instance="$2" question="$3"; shift 3
  sjbis ask \
    --question "$question" \
    --agent-name "$agent" --instance "$instance" \
    --deadline "$DEADLINE" --blocking --json \
    "$@" 2>/dev/null | sed -n '/^{/,$p'
}

field() { python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get(sys.argv[1]) or "")' "$1"; }

# resolve <json> <default> <reason>  -> effective answer (records auto-pick on timeout)
resolve() {
  local json="$1" default="$2" reason="$3" via id ans
  via=$(printf '%s' "$json" | field via)
  id=$(printf '%s' "$json" | field id)
  ans=$(printf '%s' "$json" | field answer)
  if [ "$via" = "timed_out" ] || [ -z "$ans" ]; then
    auto "$default  ($reason)"
    sjbis answer "$id" --answer "$default" --via caller-timeout \
      --note "No reply in ${DEADLINE} — $reason" >/dev/null 2>&1
    printf '%s' "$default"
  else
    human "$ans"
    printf '%s' "$ans"
  fi
}

# fire-and-forget notification (no blocking) — e.g. an inbound email
notify_async() {
  local agent="$1" instance="$2" question="$3"; shift 3
  sjbis ask --question "$question" --agent-name "$agent" --instance "$instance" \
    "$@" >/dev/null 2>&1 &
}

# ════════════════════════════════════════════════════════════════════════════
#  AGENT A — "PyForge" : building a Python-based AI system (RAG)
# ════════════════════════════════════════════════════════════════════════════
pyforge() {
  local A=PyForge I="rag-service"

  step "$A" "Designing the RAG pipeline — choose the embedding model"
  local j emb
  j=$(ask "$A" "$I" "RAG service: which embedding model for the document index?" \
        --choices "text-embedding-3-large,bge-large-en,nomic-embed,e5-mistral" --urgency 3 \
        --detail-markdown "Tradeoffs:\n- **text-embedding-3-large** — strong, hosted, \$/1M tokens\n- **bge-large-en** — great OSS, self-host\n- **nomic-embed** — small + fast\n- **e5-mistral** — top recall, heavy\n\nThis decides vector dim, the store config, and chunking below.")
  emb=$(resolve "$j" "bge-large-en" "strong open-source default we can self-host")

  step "$A" "Pick the vector store"
  j=$(ask "$A" "$I" "Which vector store should back the index for ${emb}?" \
        --choices "pgvector,Qdrant,Pinecone,Weaviate" --urgency 3 \
        --detail "pgvector keeps everything in Postgres (one less service); Qdrant/Weaviate are purpose-built; Pinecone is hosted.")
  local store; store=$(resolve "$j" "pgvector" "reuse our Postgres, fewest moving parts for v1")

  step "$A" "Chunking strategy for ingestion"
  j=$(ask "$A" "$I" "How should we chunk documents before embedding?" \
        --choices "fixed-512,semantic,recursive-markdown,sentence-window" --urgency 2 \
        --detail "Affects retrieval quality and token cost. Recursive-markdown respects headings; semantic is best but slower to ingest.")
  local chunk; chunk=$(resolve "$j" "recursive-markdown" "respects doc structure, good quality/cost balance")

  step "$A" "Eval gate before shipping the new retriever"
  j=$(ask "$A" "$I" "Min retrieval hit-rate (eval set) required to ship this config?" \
        --number --min 50 --max 99 --step 1 --default 85 --unit "%" --urgency 4 \
        --detail "We block deploy if the offline eval (recall@5 on the golden set) falls under this threshold.")
  local gate; gate=$(resolve "$j" "85" "conservative quality bar before shipping a retriever")
  bold "\n[PyForge] RAG: embed=${emb} · store=${store} · chunking=${chunk} · ship-gate=${gate}%"
}

# ════════════════════════════════════════════════════════════════════════════
#  AGENT B — "Vercelle" : deploying a frontend (path-dependent on host choice)
# ════════════════════════════════════════════════════════════════════════════
vercelle() {
  local A=Vercelle I="web-frontend"

  step "$A" "Deploying the frontend — pick the hosting target"
  local j host
  j=$(ask "$A" "$I" "Frontend deploy: where should we host the production build?" \
        --choices "Vercel,Cloudflare-Pages,AWS-S3+CloudFront,self-hosted-nginx" --urgency 3 \
        --detail-markdown "This branches the **entire** rest of the deploy:\n- **Vercel/CF** — edge + preview URLs built in\n- **S3+CloudFront** — we own cache invalidation\n- **nginx** — full control, manual everything")
  host=$(resolve "$j" "Vercel" "fastest path with preview URLs out of the box")

  if [ "$host" = "Vercel" ] || [ "$host" = "Cloudflare-Pages" ]; then
    # ── Branch 1: managed edge host → preview + edge config ──────────────────
    step "$A" "${host} chosen — generate a preview URL for every PR?"
    j=$(ask "$A" "$I" "${host}: spin up a preview deployment on every pull request?" \
          --yesno --urgency 2 \
          --detail "Preview-per-PR is great for review but multiplies build minutes.")
    local prev; prev=$(resolve "$j" "Yes" "preview-per-PR is the whole point of a managed edge host")

    step "$A" "Render mode for the app on ${host}"
    j=$(ask "$A" "$I" "Since we're on ${host}, which render mode for the routes?" \
          --choices "static-SSG,edge-SSR,ISR" --urgency 3 \
          --detail "SSG is cheapest/fastest; edge-SSR for per-request data; ISR for the middle ground.")
    local mode; mode=$(resolve "$j" "ISR" "ISR balances freshness and cost on an edge host")
    bold "\n[Vercelle] deploy: host=${host} · preview-per-PR=${prev} · render=${mode}"

  elif [ "$host" = "AWS-S3+CloudFront" ]; then
    # ── Branch 2: self-managed CDN → cache invalidation is the hard part ─────
    step "$A" "S3+CloudFront chosen — cache invalidation strategy on deploy?"
    j=$(ask "$A" "$I" "On each deploy, how do we bust the CloudFront cache?" \
          --choices "hashed-filenames,wildcard-invalidation,versioned-paths" --urgency 3 \
          --detail "Wildcard invalidations cost money and are slow; hashed filenames let us cache-forever the assets.")
    local cache; cache=$(resolve "$j" "hashed-filenames" "immutable hashed assets avoid invalidation cost entirely")

    step "$A" "Given hashed assets (${cache}), what TTL on index.html?"
    j=$(ask "$A" "$I" "With ${cache}, what Cache-Control max-age for index.html itself?" \
          --number --min 0 --max 3600 --step 30 --default 60 --unit "s" --urgency 2 \
          --detail "index.html points at the hashed bundles, so it must be short-lived to pick up new releases.")
    local ttl; ttl=$(resolve "$j" "60" "short TTL on the entry doc so releases roll out promptly")
    bold "\n[Vercelle] deploy: host=AWS-S3+CloudFront · cache=${cache} · index-ttl=${ttl}s"

  else
    # ── Branch 3: self-hosted nginx → we own TLS + rollout ───────────────────
    step "$A" "self-hosted nginx chosen — how do we cut over to the new build?"
    j=$(ask "$A" "$I" "nginx deploy: how should we switch traffic to the new build?" \
          --choices "symlink-swap,blue-green,rsync-in-place" --urgency 3 \
          --detail "symlink-swap is atomic and instantly reversible; rsync-in-place risks serving half-written files.")
    local cut; cut=$(resolve "$j" "symlink-swap" "atomic + instantly reversible cutover")

    step "$A" "After a ${cut} cutover, keep how many old releases for rollback?"
    j=$(ask "$A" "$I" "With ${cut}, how many previous releases to retain for fast rollback?" \
          --number --min 1 --max 20 --step 1 --default 5 --unit "releases" --urgency 1)
    local keep; keep=$(resolve "$j" "5" "enough history to roll back without hoarding disk")
    bold "\n[Vercelle] deploy: host=self-hosted-nginx · cutover=${cut} · keep=${keep} releases"
  fi
}

# ════════════════════════════════════════════════════════════════════════════
#  Run the demo: stagger an inbound email + both agents concurrently
# ════════════════════════════════════════════════════════════════════════════
bold "SJBIS demo — two agents building software, live (deadline ${DEADLINE})"
note "PyForge = Python AI/RAG system · Vercelle = frontend deploy"
note "Answer cards on the dashboard, or let them auto-pick on timeout."

# Inbound email lands first as a low-urgency ack card for visual variety.
notify_async Postmaster "Gmail · DoorDash" \
  "Lunch is on the way — arriving in ~8 min" \
  --ack --urgency 2 \
  --detail-markdown "**From:** orders@doordash.com\n**Subject:** Your order is on the way 🍜\n\n> Your order from **Noodle Bar** is arriving in ~8 minutes. Dasher: Marco. Drop-off: front desk."

sleep 1

# Both agents work at the same time so multiple cards are live on screen.
pyforge &
APID=$!
sleep 2          # slight stagger so the cards arrive in a readable cascade
vercelle &
NPID=$!

wait "$APID" "$NPID"
bold "\nDemo complete. Check /history on the dashboard for the full trail."
