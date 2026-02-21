#!/bin/bash
# Dogfood Learning Mesh — Arkavo Edge Self-Improvement
#
# Runs an overnight loop where specialized agents scan crates,
# write tests, and accumulate improvements on a git branch.
# Produces a PR for morning human review.
#
# Usage:
#   ./launch.sh              # Start agents + run loop
#   ./launch.sh start        # Start agents only (no loop)
#   ./launch.sh run          # Run loop (agents must be running)
#   ./launch.sh stop         # Stop all agents
#   ./launch.sh status       # Check agent health
#   ./launch.sh pr           # Create PR from current branch

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
if [ ! -f "$BINARY" ]; then
    BINARY="${PROJECT_ROOT}/target/release/arkavo"
fi
LOG_DIR="$SCRIPT_DIR/logs"
PID_FILE="$SCRIPT_DIR/.agent_pids"
RESPONSE_DIR="$SCRIPT_DIR/responses"

# Configuration
BRANCH="dogfood/$(date +%Y-%m-%d)"
MAX_CYCLES="${MAX_CYCLES:-50}"
CYCLE_INTERVAL="${CYCLE_INTERVAL:-300}"  # 5 minutes
AGENT_STARTUP_WAIT=5

# Crate rotation: smallest and most isolated first.
# Core crates (router, server, cli, gossip) are excluded.
CRATES=(
    arkavo-test-macros
    arkavo-validation
    arkavo-config-encryption
    arkavo-events
    arkavo-bench
    arkavo-sbe
    arkavo-observability
    arkavo-budget
)

# Counters
CYCLE_COUNT=0
COMMIT_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
QUALITY_SUM=0

mkdir -p "$LOG_DIR" "$RESPONSE_DIR"

print_status() {
    local status=$1
    local message=$2
    case $status in
        "INFO")    echo -e "${BLUE}[INFO]${NC} $message" ;;
        "SUCCESS") echo -e "${GREEN}[OK]${NC} $message" ;;
        "ERROR")   echo -e "${RED}[ERROR]${NC} $message" ;;
        "WARNING") echo -e "${YELLOW}[WARN]${NC} $message" ;;
        "AGENT")   echo -e "${CYAN}[AGENT]${NC} $message" ;;
        "CYCLE")   echo -e "${CYAN}[CYCLE]${NC} $message" ;;
    esac
}

check_prerequisites() {
    if [ ! -f "$BINARY" ]; then
        print_status "ERROR" "Arkavo binary not found. Run: cargo build"
        return 1
    fi
    print_status "SUCCESS" "Arkavo binary found"

    if ! command -v jq &> /dev/null; then
        print_status "ERROR" "jq is required. Install: brew install jq"
        return 1
    fi
    print_status "SUCCESS" "jq available"

    local hf_cache="${HF_HOME:-$HOME/.cache/huggingface}/hub"
    if [ -d "$hf_cache/models--unsloth--GLM-4.7-Flash-GGUF" ]; then
        print_status "SUCCESS" "GLM-4.7-Flash model available"
    else
        print_status "WARNING" "GLM-4.7-Flash not found. Run: arkavo model download glm"
    fi
}

# --- Agent lifecycle ---

stop_agents() {
    print_status "INFO" "Stopping dogfood mesh..."
    if [ -f "$PID_FILE" ]; then
        while IFS= read -r pid; do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
            fi
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi
    pkill -f "arkavo agent" 2>/dev/null || true
    print_status "SUCCESS" "All agents stopped"
}

start_agent() {
    local name=$1
    local dir=$2
    local port=$3

    if [ ! -f "$dir/AGENTS.md" ]; then
        print_status "WARNING" "No AGENTS.md in $dir, skipping $name"
        return 0
    fi

    print_status "AGENT" "Starting $name (port $port)..."

    cd "$dir"
    RUST_LOG=info nohup "$BINARY" agent run > "$LOG_DIR/${name}.log" 2>&1 &
    echo "$!" >> "$PID_FILE"
    cd "$SCRIPT_DIR"
}

start_agents() {
    > "$PID_FILE"

    start_agent "orchestrator" "$SCRIPT_DIR/orchestrator" 8420
    sleep 1
    start_agent "code-reviewer" "$SCRIPT_DIR/code-reviewer" 8422
    start_agent "test-writer" "$SCRIPT_DIR/test-writer" 8424

    print_status "INFO" "Waiting ${AGENT_STARTUP_WAIT}s for agents to initialize and discover peers..."
    sleep "$AGENT_STARTUP_WAIT"
}

show_status() {
    echo ""
    echo "Dogfood Mesh Status:"
    echo "===================="

    local agents=("orchestrator:8421" "code-reviewer:8423" "test-writer:8425")

    for agent in "${agents[@]}"; do
        local name="${agent%:*}"
        local port="${agent#*:}"
        if curl -s "http://localhost:$port/.well-known/agent.json" > /dev/null 2>&1; then
            print_status "SUCCESS" "$name (port $port)"
        else
            print_status "WARNING" "$name (port $port) - initializing..."
        fi
    done
}

# --- Git operations ---

create_branch() {
    cd "$PROJECT_ROOT"
    local current_branch
    current_branch=$(git branch --show-current)

    if [ "$current_branch" = "$BRANCH" ]; then
        print_status "INFO" "Already on branch $BRANCH"
        return 0
    fi

    if git show-ref --verify --quiet "refs/heads/$BRANCH" 2>/dev/null; then
        print_status "INFO" "Branch $BRANCH exists, switching to it"
        git checkout "$BRANCH"
    else
        print_status "INFO" "Creating branch $BRANCH"
        git checkout -b "$BRANCH"
    fi
}

create_pr() {
    cd "$PROJECT_ROOT"

    local base_branch
    base_branch=$(git branch --show-current)
    local commit_count
    commit_count=$(git rev-list --count "HEAD" --not --remotes 2>/dev/null || echo "0")

    if [ "$commit_count" -eq 0 ]; then
        # Count commits since branch diverged from parent
        commit_count=$(git log --oneline "$BRANCH" --not main 2>/dev/null | wc -l | tr -d ' ')
    fi

    if [ "$commit_count" -eq 0 ]; then
        print_status "INFO" "No changes to submit"
        return 0
    fi

    local avg_quality="0"
    if [ "$CYCLE_COUNT" -gt 0 ]; then
        avg_quality=$(echo "scale=2; $QUALITY_SUM / $CYCLE_COUNT" | bc 2>/dev/null || echo "N/A")
    fi

    local commit_log
    commit_log=$(git log --oneline "$BRANCH" --not main 2>/dev/null || echo "No commits")

    print_status "INFO" "Pushing branch $BRANCH..."
    git push origin "$BRANCH" 2>/dev/null || git push --set-upstream origin "$BRANCH"

    print_status "INFO" "Creating pull request..."
    gh pr create \
        --title "Dogfood: $(date +%Y-%m-%d) automated improvements" \
        --body "$(cat <<EOF
## Dogfood Mesh Overnight Run: $(date +%Y-%m-%d)

### Summary
$commit_count automated improvements across ${#CRATES[@]} crates.

### Changes
$commit_log

### Quality Metrics
- Cycles completed: $CYCLE_COUNT
- Successful commits: $COMMIT_COUNT
- Failed validations: $FAIL_COUNT
- Skipped (no output): $SKIP_COUNT
- Average quality score: $avg_quality

### Safety
- All changes pass \`cargo build\`, \`cargo test\`, \`cargo clippy\`
- No changes to core routing or security crates
- Human review required before merge

Generated by dogfood learning mesh.
EOF
)" \
        --base main \
        --head "$BRANCH"

    print_status "SUCCESS" "PR created"
}

# --- Task loop ---

scan_and_dispatch() {
    local crate=$1
    local cycle=$2

    print_status "CYCLE" "[$cycle/$MAX_CYCLES] Scanning $crate..."

    # Generate scan report
    local scan_file="$RESPONSE_DIR/scan_${crate}_${cycle}.json"
    if ! "$SCRIPT_DIR/scan_crate.sh" "$crate" > "$scan_file" 2>>"$LOG_DIR/scan.log"; then
        print_status "WARNING" "Scan failed for $crate"
        return 1
    fi

    # Count findings
    local warning_count
    warning_count=$(jq '.clippy_warnings | length' "$scan_file" 2>/dev/null || echo "0")
    local fn_count
    fn_count=$(jq '.public_functions | length' "$scan_file" 2>/dev/null || echo "0")
    local test_count
    test_count=$(jq '.test_list | length' "$scan_file" 2>/dev/null || echo "0")

    print_status "INFO" "  Clippy warnings: $warning_count | Public fns: $fn_count | Existing tests: $test_count"

    # Build the task prompt from scan results
    local scan_content
    scan_content=$(cat "$scan_file")

    local task_prompt="Analyze the $crate crate in the Arkavo Edge project.

Scan results:
$scan_content

Based on these findings, identify the most impactful test gaps and produce
unit tests that improve coverage for the functions listed above.
Focus on edge cases, error paths, and boundary conditions."

    # Submit task to orchestrator
    local response_file="$RESPONSE_DIR/response_${crate}_${cycle}.txt"

    print_status "INFO" "  Dispatching to orchestrator..."
    if curl -s -X POST "http://localhost:8421/tasks" \
        -H "Content-Type: application/json" \
        -d "$(jq -n --arg prompt "$task_prompt" --arg category "test_generation" \
            '{id: "dogfood-\($category)-cycle", category: $category, prompt: $prompt}')" \
        -o "$response_file" \
        --max-time 120 2>>"$LOG_DIR/dispatch.log"; then

        print_status "INFO" "  Got response ($(wc -c < "$response_file") bytes)"
    else
        print_status "WARNING" "  No response from orchestrator"
        echo "" > "$response_file"
    fi

    return 0
}

validate_and_commit() {
    local crate=$1
    local cycle=$2
    local agent_type=$3

    local response_file="$RESPONSE_DIR/response_${crate}_${cycle}.txt"

    if [ ! -s "$response_file" ]; then
        print_status "WARNING" "  Empty response, skipping validation"
        SKIP_COUNT=$((SKIP_COUNT + 1))
        return 2
    fi

    print_status "INFO" "  Validating $agent_type output..."

    local exit_code=0
    "$SCRIPT_DIR/validate.sh" "$crate" "$response_file" "$agent_type" || exit_code=$?

    case $exit_code in
        0)
            print_status "SUCCESS" "  Validation passed, changes committed"
            COMMIT_COUNT=$((COMMIT_COUNT + 1))
            QUALITY_SUM=$((QUALITY_SUM + 1))
            ;;
        1)
            print_status "WARNING" "  Validation failed, changes reverted"
            FAIL_COUNT=$((FAIL_COUNT + 1))
            ;;
        2)
            print_status "INFO" "  No actionable output"
            SKIP_COUNT=$((SKIP_COUNT + 1))
            ;;
    esac

    return "$exit_code"
}

run_loop() {
    print_status "INFO" "Starting dogfood loop: $MAX_CYCLES cycles, ${CYCLE_INTERVAL}s interval"
    print_status "INFO" "Crate rotation: ${CRATES[*]}"
    echo ""

    for ((i = 0; i < MAX_CYCLES; i++)); do
        CYCLE_COUNT=$((CYCLE_COUNT + 1))
        local crate_idx=$((i % ${#CRATES[@]}))
        local crate="${CRATES[$crate_idx]}"

        scan_and_dispatch "$crate" "$CYCLE_COUNT" || true
        validate_and_commit "$crate" "$CYCLE_COUNT" "test-writer" || true

        # Progress summary
        print_status "CYCLE" "Progress: $COMMIT_COUNT committed, $FAIL_COUNT failed, $SKIP_COUNT skipped"
        echo ""

        # Sleep between cycles (unless this is the last one)
        if [ "$i" -lt $((MAX_CYCLES - 1)) ]; then
            print_status "INFO" "Sleeping ${CYCLE_INTERVAL}s until next cycle..."
            sleep "$CYCLE_INTERVAL"
        fi
    done

    print_status "SUCCESS" "Loop complete: $CYCLE_COUNT cycles"
}

# --- Cleanup ---

cleanup() {
    print_status "INFO" "Cleaning up..."
    # Remove any leftover git worktrees
    cd "$PROJECT_ROOT"
    git worktree list --porcelain 2>/dev/null | grep '^worktree' | grep 'dogfood' | while read -r _ path; do
        git worktree remove "$path" --force 2>/dev/null || true
    done
    stop_agents
}

trap cleanup EXIT INT TERM

# --- Main ---

main() {
    echo ""
    echo "=========================================================="
    echo "  ARKAVO - Dogfood Learning Mesh"
    echo "  Self-Improving Codebase via Thompson Sampling"
    echo "=========================================================="
    echo ""

    case "${1:-default}" in
        stop)
            trap - EXIT INT TERM  # Don't double-cleanup
            stop_agents
            ;;
        restart)
            stop_agents
            sleep 2
            check_prerequisites || exit 1
            start_agents
            show_status
            ;;
        status)
            show_status
            ;;
        start)
            check_prerequisites || exit 1
            stop_agents
            start_agents
            show_status
            echo ""
            print_status "INFO" "Logs: $LOG_DIR/"
            print_status "INFO" "Stop: $0 stop"
            print_status "INFO" "Run loop: $0 run"
            print_status "INFO" "UI: cargo run -p arkavo -- ui 7700"
            ;;
        run)
            create_branch
            run_loop
            print_status "INFO" "Creating PR..."
            create_pr
            ;;
        pr)
            create_pr
            ;;
        default)
            check_prerequisites || exit 1
            stop_agents
            create_branch
            start_agents
            show_status
            echo ""
            print_status "INFO" "Logs: $LOG_DIR/"
            print_status "INFO" "Stop: $0 stop"
            echo ""
            print_status "INFO" "Starting task loop in 10s (Ctrl+C to cancel)..."
            sleep 10
            run_loop
            print_status "INFO" "Creating PR..."
            create_pr
            ;;
        *)
            echo "Usage: $0 [start|stop|restart|status|run|pr]"
            echo ""
            echo "Commands:"
            echo "  (default)  Start agents + run loop + create PR"
            echo "  start      Start agents only"
            echo "  run        Run task loop (agents must be running)"
            echo "  stop       Stop all agents"
            echo "  status     Check agent health"
            echo "  pr         Create PR from current branch"
            echo ""
            echo "Environment:"
            echo "  MAX_CYCLES=50        Number of cycles (default: 50)"
            echo "  CYCLE_INTERVAL=300   Seconds between cycles (default: 300)"
            echo "  BINARY=path/to/arkavo  Custom binary path"
            exit 1
            ;;
    esac
}

main "$@"
