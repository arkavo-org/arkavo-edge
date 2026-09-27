#!/bin/bash
# Shadow Teacher Kit — gather real episode traces with local models, then
# preview the shadow-consolidation batch with a zero-spend dry run.
#
# Stages:
#   1. Validate and launch the kit manifest (plane separation included)
#   2. Start the two worker agents with local models
#   3. Submit the tool-driven tasks from tasks.json
#   4. Wait for episodes to land in the learning stores
#   5. Merge the stores and run arkavo-shadow-consolidation --dry-run

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
if [ ! -f "$BINARY" ]; then
    BINARY="${PROJECT_ROOT}/target/release/arkavo"
fi
LOG_DIR="$SCRIPT_DIR/logs"
OUT_DIR="$SCRIPT_DIR/out"
PID_FILE="$SCRIPT_DIR/.agent_pids"
MANIFEST="$SCRIPT_DIR/shadow-teacher-kit.swarmkit.yaml"
TASKS_FILE="$SCRIPT_DIR/tasks.json"
AGENTS=("code-reviewer:8430" "test-writer:8432")
# Local inference is slow; allow generous per-task and episode waits.
TASK_TIMEOUT="${TASK_TIMEOUT:-420}"
EPISODE_WAIT="${EPISODE_WAIT:-300}"
SQLITE="${SQLITE:-sqlite3}"

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; BLUE='\033[0;34m'; NC='\033[0m'
info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
ok()      { echo -e "${GREEN}[OK]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
fail()    { echo -e "${RED}[ERROR]${NC} $1"; }

mkdir -p "$LOG_DIR" "$OUT_DIR"

check_prerequisites() {
    [ -f "$BINARY" ] || { fail "arkavo binary not found — run: cargo build"; exit 1; }
    ok "arkavo binary: $BINARY"
    for cmd in jq curl "$SQLITE"; do
        command -v "$cmd" >/dev/null || { fail "$cmd is required"; exit 1; }
    done
    local hf_cache="${HF_HOME:-$HOME/.cache/huggingface}/hub"
    if [ -d "$hf_cache/models--ggml-org--gemma-4-E4B-it-GGUF" ]; then
        ok "gemma-4-e4b model cached"
    else
        warn "gemma-4-e4b not cached; first task will be slow or fail."
        warn "Download: hf download ggml-org/gemma-4-E4B-it-GGUF gemma-4-E4B-it-Q4_K_M.gguf"
    fi
}

validate_kit() {
    info "Validating kit manifest (parse + flight launch + per-role ARP)..."
    (cd "$PROJECT_ROOT" && cargo run -q -p arkavo-swarmkit-runtime \
        --example swarm_flight_demo -- "$MANIFEST") \
        > "$LOG_DIR/kit-validation.log" 2>&1 \
        || { fail "kit manifest failed validation — see logs/kit-validation.log"; exit 1; }
    ok "kit manifest valid; flight launched (see logs/kit-validation.log)"
}

stop_agents() {
    if [ -f "$PID_FILE" ]; then
        while IFS= read -r pid; do
            kill "$pid" 2>/dev/null || true
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi
    # Belt and suspenders: an agent that survives across runs keeps its
    # ports and an open handle to a learning store we are about to reset,
    # which then fails writes with SQLITE_READONLY_DBMOVED. Kill any
    # stragglers and wait for the ports to actually free.
    pkill -f "$BINARY agent run" 2>/dev/null || true
    for entry in "${AGENTS[@]}"; do
        local port="${entry#*:}"
        for _ in $(seq 1 20); do
            lsof -iTCP:"$port" -sTCP:LISTEN -n >/dev/null 2>&1 || break
            sleep 0.5
        done
    done
}

start_agents() {
    : > "$PID_FILE"
    for entry in "${AGENTS[@]}"; do
        local name="${entry%:*}" port="${entry#*:}"
        local dir="$SCRIPT_DIR/$name"
        # Each agent reads the sample files from its own workspace so the
        # filesystem tools operate inside the agent's working directory.
        mkdir -p "$dir/workspace"
        cp "$SCRIPT_DIR"/sample/*.rs "$dir/workspace/"
        info "Starting $name (port $port)..."
        (cd "$dir" && RUST_LOG=info nohup "$BINARY" agent run \
            > "$LOG_DIR/$name.log" 2>&1 & echo $! >> "$PID_FILE")
    done

    info "Waiting for agents to come up..."
    for entry in "${AGENTS[@]}"; do
        local name="${entry%:*}" port="${entry#*:}"
        for _ in $(seq 1 60); do
            if curl -sf "http://localhost:$port/.well-known/agent.json" >/dev/null 2>&1; then
                ok "$name ready on port $port"
                continue 2
            fi
            sleep 2
        done
        fail "$name never became ready — see logs/$name.log"
        exit 1
    done
}

submit_tasks() {
    local count
    count=$(jq '.tasks | length' "$TASKS_FILE")
    info "Submitting $count tool-driven tasks (sequential; local inference takes minutes per task)..."
    for i in $(seq 0 $((count - 1))); do
        local id prompt port
        id=$(jq -r ".tasks[$i].id" "$TASKS_FILE")
        prompt=$(jq -r ".tasks[$i].prompt" "$TASKS_FILE")
        port=$(jq -r ".tasks[$i].port" "$TASKS_FILE")
        info "Task $((i + 1))/$count [$id] -> port $port"
        local response
        response=$(curl -sf --max-time "$TASK_TIMEOUT" -X POST "http://localhost:$port" \
            -H "Content-Type: application/json" \
            -d "$(jq -n --arg t "$prompt" '{
                jsonrpc: "2.0", id: 1, method: "message/send",
                params: { request: { message: { parts: [{ type: "text", content: $t }] } } }
            }')" || echo "")
        if [ -n "$response" ] && ! echo "$response" | jq -e '.error' >/dev/null 2>&1; then
            ok "task $id completed"
        else
            warn "task $id failed or timed out (continuing): ${response:0:160}"
        fi
    done
}

episode_count() {
    local total=0
    for entry in "${AGENTS[@]}"; do
        local name="${entry%:*}"
        local db="$SCRIPT_DIR/$name/.arkavo/learning/lessons.db"
        if [ -f "$db" ]; then
            local n
            n=$("$SQLITE" "file:$db?mode=ro" \
                "SELECT COUNT(*) FROM episodes" 2>/dev/null || echo 0)
            total=$((total + n))
        fi
    done
    echo "$total"
}

wait_for_episodes() {
    info "Waiting for episode synthesis (threshold: 2 observations per category)..."
    local waited=0
    while [ "$waited" -lt "$EPISODE_WAIT" ]; do
        local n
        n=$(episode_count)
        if [ "$n" -ge 2 ]; then
            ok "$n episodes recorded"
            return 0
        fi
        sleep 10
        waited=$((waited + 10))
    done
    warn "Only $(episode_count) episodes after ${EPISODE_WAIT}s — continuing with what exists"
}

merge_stores() {
    local merged="$OUT_DIR/episodes.db"
    rm -f "$merged"
    "$SQLITE" "$merged" "CREATE TABLE episodes (id TEXT PRIMARY KEY, \
        task_category TEXT NOT NULL, observation_json TEXT NOT NULL, \
        outcome_json TEXT NOT NULL)"
    for entry in "${AGENTS[@]}"; do
        local name="${entry%:*}"
        local db="$SCRIPT_DIR/$name/.arkavo/learning/lessons.db"
        [ -f "$db" ] || continue
        "$SQLITE" "$merged" "ATTACH '$db' AS src; \
            INSERT OR IGNORE INTO episodes \
            SELECT id, task_category, observation_json, outcome_json \
            FROM src.episodes; DETACH src;" 2>/dev/null || true
    done
    local n
    n=$("$SQLITE" "$merged" "SELECT COUNT(*) FROM episodes")
    ok "merged store: $n episodes -> out/episodes.db"
    "$SQLITE" "$merged" "SELECT task_category, COUNT(*) FROM episodes GROUP BY task_category" \
        | sed 's/^/    /'
}

run_shadow_dry() {
    info "Running shadow-consolidation dry run (zero spend, no API key)..."
    (cd "$PROJECT_ROOT" && cargo run -q -p arkavo-shadow-consolidation -- \
        --db "$OUT_DIR/episodes.db" \
        --out "$OUT_DIR/shadow" \
        --min-episodes 2 \
        --dry-run) | sed 's/^/    /'
    echo ""
    ok "Artifacts in out/shadow/ — prompts.json shows exactly what Fable would receive."
    info "Live run (real spend, capped): cargo run -p arkavo-shadow-consolidation -- \\"
    info "  --db examples/shadow-teacher-kit/out/episodes.db --out .../shadow-live --max-run-cost-usd 1.0"
}

main() {
    echo "=========================================================="
    echo "  Shadow Teacher Kit — episode gathering with local models"
    echo "=========================================================="
    check_prerequisites
    validate_kit
    stop_agents
    start_agents
    submit_tasks
    wait_for_episodes
    stop_agents
    merge_stores
    run_shadow_dry
}

main "$@"
