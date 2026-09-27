#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
FIXTURE_ROOT="$REPO_ROOT/tests/fixtures/triage"
RUN_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/sjbis-triage-e2e.XXXXXX")
PG_DATA="$RUN_ROOT/postgres"
PG_SOCKET="$RUN_ROOT/postgres-socket"
TEST_HOME="$RUN_ROOT/home"
DAEMON_LOG="$RUN_ROOT/daemon.log"
POSTGRES_LOG="$RUN_ROOT/postgres.log"
DAEMON_PID=""
POSTGRES_STARTED=0

fail() {
    echo "triage-e2e: $*" >&2
    exit 1
}

cleanup() {
    if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    if [[ "$POSTGRES_STARTED" == 1 ]]; then
        "$PG_CTL" -D "$PG_DATA" -m immediate -w stop >/dev/null 2>&1 || true
    fi
    if [[ "${SJBIS_TRIAGE_E2E_KEEP:-0}" == 1 ]]; then
        echo "triage-e2e: retained $RUN_ROOT"
    else
        rm -rf -- "$RUN_ROOT"
    fi
}
trap cleanup EXIT INT TERM

for tool in cargo curl jq python3 pg_config; do
    command -v "$tool" >/dev/null 2>&1 || fail "required tool is missing: $tool"
done

PG_BINDIR=${PG_BINDIR:-$(pg_config --bindir)}
INITDB=${INITDB:-$PG_BINDIR/initdb}
PG_CTL=${PG_CTL:-$PG_BINDIR/pg_ctl}
CREATEDB=${CREATEDB:-$PG_BINDIR/createdb}
for tool in "$INITDB" "$PG_CTL" "$CREATEDB"; do
    [[ -x "$tool" ]] || fail "required PostgreSQL executable is missing: $tool"
done

free_port() {
    python3 - <<'PY'
import socket

with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    print(listener.getsockname()[1])
PY
}

PG_PORT=$(free_port)
DAEMON_PORT=$(free_port)
while [[ "$DAEMON_PORT" == "$PG_PORT" ]]; do
    DAEMON_PORT=$(free_port)
done
DAEMON_URL="http://127.0.0.1:$DAEMON_PORT"
DATABASE_NAME="sjbis_triage_e2e"

mkdir -p "$PG_SOCKET" "$TEST_HOME/.config/sjbis"
"$INITDB" -D "$PG_DATA" --auth=trust --username=postgres --no-locale --encoding=UTF8 >/dev/null
"$PG_CTL" -D "$PG_DATA" -l "$POSTGRES_LOG" \
    -o "-F -h 127.0.0.1 -k $PG_SOCKET -p $PG_PORT" -w start >/dev/null
POSTGRES_STARTED=1
"$CREATEDB" -h 127.0.0.1 -p "$PG_PORT" -U postgres "$DATABASE_NAME"
printf '[database]\ndsn = "postgresql://postgres@127.0.0.1:%s/%s"\n' \
    "$PG_PORT" "$DATABASE_NAME" >"$TEST_HOME/.config/sjbis/database.toml"

if [[ -n "${SJBIS_BIN:-}" ]]; then
    BIN=$SJBIS_BIN
else
    cargo build --quiet --bin sjbis --manifest-path "$REPO_ROOT/Cargo.toml"
    BIN="$REPO_ROOT/target/debug/sjbis"
fi
[[ -x "$BIN" ]] || fail "sjbis binary is not executable: $BIN"

(
    cd "$REPO_ROOT"
    HOME="$TEST_HOME" RUST_LOG=warn "$BIN" daemon start --port "$DAEMON_PORT"
) >"$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!

for _ in $(seq 1 100); do
    if curl --silent --fail "$DAEMON_URL/health" >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
        sed -n '1,200p' "$DAEMON_LOG" >&2
        fail "daemon exited before becoming healthy"
    fi
    sleep 0.1
done
curl --silent --fail "$DAEMON_URL/health" >/dev/null || {
    sed -n '1,200p' "$DAEMON_LOG" >&2
    fail "daemon did not become healthy"
}

cli() {
    HOME="$TEST_HOME" SJBIS_DAEMON="$DAEMON_URL" "$BIN" "$@"
}

api_json() {
    local method=$1
    local path=$2
    local body=${3-}
    if [[ -n "$body" ]]; then
        curl --silent --show-error --fail-with-body \
            -X "$method" -H 'Content-Type: application/json' -d "$body" \
            "$DAEMON_URL$path"
    else
        curl --silent --show-error --fail-with-body -X "$method" "$DAEMON_URL$path"
    fi
}

assert_http_status() {
    local expected=$1
    local method=$2
    local path=$3
    local body=${4-}
    local response="$RUN_ROOT/http-response.json"
    local status
    if [[ -n "$body" ]]; then
        status=$(curl --silent --show-error -o "$response" -w '%{http_code}' \
            -X "$method" -H 'Content-Type: application/json' -d "$body" \
            "$DAEMON_URL$path")
    else
        status=$(curl --silent --show-error -o "$response" -w '%{http_code}' \
            -X "$method" "$DAEMON_URL$path")
    fi
    [[ "$status" == "$expected" ]] || {
        cat "$response" >&2
        fail "$method $path returned HTTP $status, expected $expected"
    }
}

assert_cli_exit_3() {
    local label=$1
    shift
    set +e
    cli "$@" >"$RUN_ROOT/cli-closed.out" 2>"$RUN_ROOT/cli-closed.err"
    local status=$?
    set -e
    [[ "$status" == 3 ]] || {
        cat "$RUN_ROOT/cli-closed.out" >&2
        cat "$RUN_ROOT/cli-closed.err" >&2
        fail "$label exited $status, expected 3"
    }
}

cp -R "$FIXTURE_ROOT/catalog" "$RUN_ROOT/catalog"
cp -R "$FIXTURE_ROOT/duplicates" "$RUN_ROOT/duplicates"
CATALOG_ROOT="$RUN_ROOT/catalog"

set +e
cli triage create duplicate-fixture --root "$RUN_ROOT/duplicates" --glob '*.md' \
    >"$RUN_ROOT/duplicate.out" 2>"$RUN_ROOT/duplicate.err"
DUPLICATE_STATUS=$?
set -e
[[ "$DUPLICATE_STATUS" == 1 ]] || fail "duplicate identity import exited $DUPLICATE_STATUS, expected 1"
grep -q 'item identity collisions: duplicate-identity' "$RUN_ROOT/duplicate.err" || \
    fail "duplicate identity diagnostic did not name the collision"

cli triage --json create acceptance-queue --root "$CATALOG_ROOT" \
    --json-list "$CATALOG_ROOT/items.json" >"$RUN_ROOT/created.json"
QUEUE_ID=$(jq -er '.id' "$RUN_ROOT/created.json")

api_json GET "/triage/queues/$QUEUE_ID" >"$RUN_ROOT/initial.json"
jq -e '
    [.catalog[].id] == [
        "declared-alpha",
        "02-basename",
        "03-stale",
        "04-move-old",
        "05-ambiguous-old",
        "merge-source",
        "merge-target",
        "merge-terminal",
        "inline-provenance"
    ]
' "$RUN_ROOT/initial.json" >/dev/null || fail "catalog ordering or imported identities changed"
jq -e '
    .catalog[]
    | select(.id == "inline-provenance")
    | .source == {kind:"yesod_note", note_id:"ys-fixture-inline", revision:7}
' "$RUN_ROOT/initial.json" >/dev/null || fail "inline source provenance was not preserved"

cli triage --json decide "$QUEUE_ID" declared-alpha schedule >"$RUN_ROOT/schedule.json"
cli triage --json decide "$QUEUE_ID" 02-basename needs_replan >"$RUN_ROOT/basename-decision.json"
cli triage --json decide "$QUEUE_ID" merge-source merge_into --target merge-target \
    >"$RUN_ROOT/source-merge.json"
jq -e '.target == "merge-target" and .verdict == "merge_into"' \
    "$RUN_ROOT/source-merge.json" >/dev/null || fail "declared merge target was not recorded"

# The first consumer starts at origin and persists only the opaque cursor.
(
    cli triage --json export "$QUEUE_ID" --since '' --limit 100 >"$RUN_ROOT/page-one.json"
    jq -er '.nextCursor' "$RUN_ROOT/page-one.json" >"$RUN_ROOT/cursor.txt"
)

api_json PATCH "/triage/queues/$QUEUE_ID/items/declared-alpha/decision" \
    '{"verdict":"delete"}' >"$RUN_ROOT/revised.json"
cli triage --json clear "$QUEUE_ID" 02-basename >"$RUN_ROOT/cleared.json"
api_json PATCH "/triage/queues/$QUEUE_ID/items/merge-target/decision" \
    '{"verdict":"merge_into","target":"merge-terminal"}' >"$RUN_ROOT/target-merge.json"
jq -e '.revision == 2 and .verdict == "delete"' "$RUN_ROOT/revised.json" >/dev/null || \
    fail "verdict revision history did not advance"
jq -e '.revision == 2 and .verdict == null and .target == null' "$RUN_ROOT/cleared.json" >/dev/null || \
    fail "clear did not append an explicit null revision"

# A separate HTTP consumer resumes from the cursor persisted by the CLI process.
(
    CURSOR=$(<"$RUN_ROOT/cursor.txt")
    curl --silent --show-error --fail-with-body \
        "$DAEMON_URL/triage/queues/$QUEUE_ID/export?since=$CURSOR&limit=100" \
        >"$RUN_ROOT/page-two.json"
)
api_json GET "/triage/queues/$QUEUE_ID/export?since=&limit=100" >"$RUN_ROOT/origin.json"
python3 - "$RUN_ROOT/page-one.json" "$RUN_ROOT/page-two.json" "$RUN_ROOT/origin.json" <<'PY'
import json
import sys

first, resumed, origin = (json.load(open(path, encoding="utf-8")) for path in sys.argv[1:])
first_ids = [item["event_id"] for item in first["items"]]
resumed_ids = [item["event_id"] for item in resumed["items"]]
origin_ids = [item["event_id"] for item in origin["items"]]
combined = first_ids + resumed_ids
if combined != origin_ids:
    raise SystemExit(f"cursor resume skipped or reordered events: {combined!r} != {origin_ids!r}")
if len(combined) != len(set(combined)):
    raise SystemExit(f"cursor resume duplicated event ids: {combined!r}")
if not first_ids or not resumed_ids:
    raise SystemExit("cursor fixture requires events on both sides of the persisted cursor")
PY

printf '# Changed producer content\n\nThe served snapshot must remain the original.\n' \
    >"$CATALOG_ROOT/03-stale_analysis.md"
mkdir -p "$CATALOG_ROOT/moved" "$CATALOG_ROOT/ambiguous"
mv "$CATALOG_ROOT/04-move-old_analysis.md" "$CATALOG_ROOT/moved/move-new_analysis.md"
cp "$CATALOG_ROOT/05-ambiguous-old_analysis.md" \
    "$CATALOG_ROOT/ambiguous/candidate-one_analysis.md"
cp "$CATALOG_ROOT/05-ambiguous-old_analysis.md" \
    "$CATALOG_ROOT/ambiguous/candidate-two_analysis.md"
rm "$CATALOG_ROOT/05-ambiguous-old_analysis.md"
jq '
    map(
        if .path == "04-move-old_analysis.md" then
            {path:"moved/move-new_analysis.md"}
        elif .path == "05-ambiguous-old_analysis.md" then
            empty
        else
            .
        end
    ) + [
        {path:"ambiguous/candidate-one_analysis.md"},
        {path:"ambiguous/candidate-two_analysis.md"}
    ]
' "$CATALOG_ROOT/items.json" >"$RUN_ROOT/items.next.json"
mv "$RUN_ROOT/items.next.json" "$CATALOG_ROOT/items.json"

cli triage --json refresh "$QUEUE_ID" >"$RUN_ROOT/refreshed.json"
jq -e '.content_changed == 1 and .moved == 1 and .ambiguous == 1' \
    "$RUN_ROOT/refreshed.json" >/dev/null || fail "refresh did not classify stale and moved fixtures"
api_json GET "/triage/queues/$QUEUE_ID" >"$RUN_ROOT/after-refresh.json"
jq -e --slurpfile initial "$RUN_ROOT/initial.json" '
    (.catalog[] | select(.id == "03-stale")) as $refreshed
    | ($initial[0].catalog[] | select(.id == "03-stale")) as $captured
    | $refreshed.freshness.state == "content_changed"
      and $refreshed.markdown == $captured.markdown
      and $refreshed.content_sha256 == $captured.content_sha256
' "$RUN_ROOT/after-refresh.json" >/dev/null || fail "stale item lost its served snapshot"
jq -e '
    .catalog[]
    | select(.id == "04-move-old")
    | .path == "moved/move-new_analysis.md"
      and .freshness.state == "current"
      and .freshness.reason == "source_moved"
' "$RUN_ROOT/after-refresh.json" >/dev/null || fail "unambiguous hash move was not reattached"
jq -e '
    .catalog[]
    | select(.id == "05-ambiguous-old")
    | .freshness.state == "ambiguous"
      and .freshness.candidate_paths == [
          "ambiguous/candidate-one_analysis.md",
          "ambiguous/candidate-two_analysis.md"
      ]
' "$RUN_ROOT/after-refresh.json" >/dev/null || fail "ambiguous hash move candidates were not retained"

cli triage --json attach "$QUEUE_ID" 05-ambiguous-old \
    ambiguous/candidate-one_analysis.md >"$RUN_ROOT/attached.json"
jq -e '
    .id == "05-ambiguous-old"
    and .path == "ambiguous/candidate-one_analysis.md"
    and .freshness.state == "current"
' "$RUN_ROOT/attached.json" >/dev/null || fail "explicit attachment did not resolve ambiguity"

api_json GET "/triage/queues/$QUEUE_ID" >"$RUN_ROOT/direct-merges.json"
jq -e '
    ([.catalog[] | select(.id == "merge-source") | .latest_revision.target] == ["merge-target"])
    and ([.catalog[] | select(.id == "merge-target") | .latest_revision.target] == ["merge-terminal"])
' "$RUN_ROOT/direct-merges.json" >/dev/null || fail "merge targets were collapsed instead of remaining direct edges"
jq -e '.queue.complete == false and .queue.status == "open"' \
    "$RUN_ROOT/direct-merges.json" >/dev/null || fail "queue should be incomplete before freezing"

cli triage --json close "$QUEUE_ID" >"$RUN_ROOT/closed.json"
jq -e '.status == "closed" and .complete == false' "$RUN_ROOT/closed.json" >/dev/null || \
    fail "incomplete queue did not close"

ATTACH_BODY=$(jq -n --rawfile markdown "$CATALOG_ROOT/ambiguous/candidate-one_analysis.md" \
    '{path:"ambiguous/candidate-one_analysis.md", markdown:$markdown}')
assert_http_status 409 PATCH "/triage/queues/$QUEUE_ID/items/declared-alpha/decision" \
    '{"verdict":"schedule"}'
assert_http_status 409 PATCH "/triage/queues/$QUEUE_ID/items/declared-alpha/decision" \
    '{"verdict":null}'
assert_http_status 409 POST "/triage/queues/$QUEUE_ID/refresh" '{"items":[]}'
assert_http_status 409 POST "/triage/queues/$QUEUE_ID/items/05-ambiguous-old/attach" "$ATTACH_BODY"

assert_cli_exit_3 "closed decide" triage decide "$QUEUE_ID" declared-alpha schedule
assert_cli_exit_3 "closed clear" triage clear "$QUEUE_ID" declared-alpha
assert_cli_exit_3 "closed refresh" triage refresh "$QUEUE_ID"
assert_cli_exit_3 "closed attach" triage attach "$QUEUE_ID" 05-ambiguous-old \
    ambiguous/candidate-one_analysis.md

cli triage --json export "$QUEUE_ID" --since '' >"$RUN_ROOT/closed-export.json"
jq -e '.queue.status == "closed" and (.items | length) == 6' \
    "$RUN_ROOT/closed-export.json" >/dev/null || fail "closed queue export was not available"

cli triage --json reopen "$QUEUE_ID" >"$RUN_ROOT/reopened.json"
jq -e '.status == "open" and .complete == false' "$RUN_ROOT/reopened.json" >/dev/null || \
    fail "queue did not reopen"

cli triage --json show "$QUEUE_ID" >"$RUN_ROOT/before-completion.json"
while IFS= read -r item_id; do
    cli triage --json decide "$QUEUE_ID" "$item_id" leave_captured >/dev/null
done < <(jq -r '.catalog[] | select(.latest_revision == null or .latest_revision.verdict == null) | .id' \
    "$RUN_ROOT/before-completion.json")

cli triage --json show "$QUEUE_ID" >"$RUN_ROOT/completed.json"
jq -e '
    .queue.status == "open"
    and .queue.complete == true
    and .queue.counts.decided == .queue.counts.total
' "$RUN_ROOT/completed.json" >/dev/null || fail "fully decided queue was not complete and open"

cli triage --json export "$QUEUE_ID" --since '' >"$RUN_ROOT/final-export.json"
jq -e '
    [.items[] | select(.item_id == "declared-alpha") | .verdict] == ["schedule", "delete"]
    and [.items[] | select(.item_id == "02-basename") | .verdict] == ["needs_replan", null, "leave_captured"]
    and ([.items[].event_id] | length) == ([.items[].event_id] | unique | length)
' "$RUN_ROOT/final-export.json" >/dev/null || fail "final revision history is incomplete or duplicated"

echo "triage-e2e: passed ($QUEUE_ID)"
