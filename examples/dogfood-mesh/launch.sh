#!/bin/bash
# Dogfood Learning Mesh — Arkavo Edge Self-Improvement
#
# Manages the agent mesh and dispatches a single scan+task per invocation.
# The orchestrator agent handles routing, judging, and lesson extraction.
#
# Usage:
#   ./launch.sh start             # Start agents
#   ./launch.sh scan <crate>      # Scan crate + dispatch to mesh
#   ./launch.sh stop              # Stop all agents
#   ./launch.sh status            # Check agent health
#   ./launch.sh pr                # Create PR from current branch

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
AGENT_STARTUP_WAIT=5

# Safe crate targets (core crates excluded)
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

create_pr() {
    cd "$PROJECT_ROOT"

    local current_branch
    current_branch=$(git branch --show-current)
    local commit_count
    commit_count=$(git log --oneline "$current_branch" --not main 2>/dev/null | wc -l | tr -d ' ')

    if [ "$commit_count" -eq 0 ]; then
        print_status "INFO" "No changes to submit"
        return 0
    fi

    local commit_log
    commit_log=$(git log --oneline "$current_branch" --not main 2>/dev/null || echo "No commits")

    print_status "INFO" "Pushing branch $current_branch..."
    git push origin "$current_branch" 2>/dev/null || git push --set-upstream origin "$current_branch"

    print_status "INFO" "Creating pull request..."
    gh pr create \
        --title "Dogfood: $(date +%Y-%m-%d) automated improvements" \
        --body "$(cat <<EOF
## Dogfood Mesh Run: $(date +%Y-%m-%d)

### Changes
$commit_log

### Safety
- All changes pass \`cargo build\`, \`cargo test\`, \`cargo clippy\`
- No changes to core routing or security crates
- Human review required before merge

Generated by dogfood learning mesh.
EOF
)" \
        --base main \
        --head "$current_branch"

    print_status "SUCCESS" "PR created"
}

# --- Scan and dispatch ---

scan_and_dispatch() {
    local crate=$1
    local timestamp
    timestamp=$(date +%s)

    # Validate crate is in the safe list
    local found=0
    for c in "${CRATES[@]}"; do
        if [ "$c" = "$crate" ]; then found=1; break; fi
    done
    if [ "$found" -eq 0 ]; then
        print_status "ERROR" "Crate '$crate' not in safe list: ${CRATES[*]}"
        return 1
    fi

    print_status "INFO" "Scanning $crate..."

    # Generate scan report
    local scan_file="$RESPONSE_DIR/scan_${crate}_${timestamp}.json"
    if ! "$SCRIPT_DIR/scan_crate.sh" "$crate" > "$scan_file" 2>>"$LOG_DIR/scan.log"; then
        print_status "ERROR" "Scan failed for $crate"
        return 1
    fi

    # Report findings
    local warning_count fn_count test_count
    warning_count=$(jq '.clippy_warnings | length' "$scan_file" 2>/dev/null || echo "0")
    fn_count=$(jq '.public_functions | length' "$scan_file" 2>/dev/null || echo "0")
    test_count=$(jq '.test_list | length' "$scan_file" 2>/dev/null || echo "0")
    print_status "INFO" "Clippy warnings: $warning_count | Public fns: $fn_count | Existing tests: $test_count"

    # Build task prompt from scan results
    local scan_content
    scan_content=$(cat "$scan_file")

    local task_prompt="Analyze the $crate crate in the Arkavo Edge project.

Scan results:
$scan_content

Based on these findings, identify the most impactful test gaps and produce
unit tests that improve coverage for the functions listed above.
Focus on edge cases, error paths, and boundary conditions."

    # Dispatch to mesh via A2A protocol
    local response_file="$RESPONSE_DIR/response_${crate}_${timestamp}.txt"

    print_status "INFO" "Dispatching to mesh..."
    if "$BINARY" task "$task_prompt" --mesh-only --yes --no-validate \
        > "$response_file" 2>>"$LOG_DIR/dispatch.log"; then
        print_status "SUCCESS" "Got response ($(wc -c < "$response_file" | tr -d ' ') bytes)"
    else
        print_status "WARNING" "Task dispatch failed"
        echo "" > "$response_file"
    fi

    # Validate output
    if [ -s "$response_file" ]; then
        print_status "INFO" "Validating output..."
        local exit_code=0
        "$SCRIPT_DIR/validate.sh" "$crate" "$response_file" "test-writer" || exit_code=$?

        case $exit_code in
            0) print_status "SUCCESS" "Validation passed, changes committed" ;;
            1) print_status "WARNING" "Validation failed, changes reverted" ;;
            2) print_status "INFO" "No actionable output" ;;
        esac
    else
        print_status "WARNING" "Empty response"
    fi
}

# --- Main ---

main() {
    echo ""
    echo "=========================================================="
    echo "  ARKAVO - Dogfood Learning Mesh"
    echo "=========================================================="
    echo ""

    case "${1:-help}" in
        start)
            check_prerequisites || exit 1
            stop_agents
            start_agents
            show_status
            echo ""
            print_status "INFO" "Logs: $LOG_DIR/"
            print_status "INFO" "Stop: $0 stop"
            print_status "INFO" "Scan: $0 scan <crate-name>"
            print_status "INFO" "UI:   cargo run -p arkavo -- ui 7700"
            ;;
        stop)
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
        scan)
            local crate="${2:?Usage: $0 scan <crate-name>}"
            scan_and_dispatch "$crate"
            ;;
        pr)
            create_pr
            ;;
        *)
            echo "Usage: $0 <command> [args]"
            echo ""
            echo "Commands:"
            echo "  start          Start the agent mesh"
            echo "  stop           Stop all agents"
            echo "  restart        Restart the mesh"
            echo "  status         Check agent health"
            echo "  scan <crate>   Scan a crate and dispatch to mesh"
            echo "  pr             Create PR from current branch"
            echo ""
            echo "Safe crates: ${CRATES[*]}"
            echo ""
            echo "Example:"
            echo "  $0 start"
            echo "  $0 scan arkavo-validation"
            echo "  $0 scan arkavo-events"
            echo "  $0 pr"
            exit 1
            ;;
    esac
}

main "$@"
