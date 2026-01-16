#!/bin/bash
# Unified demo - runs tasks through local models with mDNS agent discovery

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${BINARY:-$SCRIPT_DIR/../target/debug/arkavo}"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

# Scenarios to run (in order)
SCENARIOS=("software-development-simple" "family-travel-mesh" "fleet-immunity")

DELAY="${1:-10}"

print_banner() {
    echo -e "${BLUE}"
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║              Arkavo Unified Demo                              ║"
    echo "╚═══════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"
}

check_binary() {
    if [ ! -f "$BINARY" ]; then
        echo -e "${RED}Binary not found at $BINARY${NC}"
        echo "Please run: cargo build"
        exit 1
    fi
}

start_orchestrator() {
    echo "Starting orchestrator..."
    cd "$SCRIPT_DIR"
    nohup "$BINARY" agent run > "$SCRIPT_DIR/logs/orchestrator.log" 2>&1 &
    echo $! > "$SCRIPT_DIR/.orchestrator_pid"
    sleep 2
    echo -e "${GREEN}Orchestrator started${NC}"
}

stop_orchestrator() {
    if [ -f "$SCRIPT_DIR/.orchestrator_pid" ]; then
        pid=$(cat "$SCRIPT_DIR/.orchestrator_pid")
        kill "$pid" 2>/dev/null || true
        rm -f "$SCRIPT_DIR/.orchestrator_pid"
    fi
}

run_scenario() {
    local scenario="$1"
    local scenario_dir="$SCRIPT_DIR/$scenario"
    local tasks_file="$scenario_dir/tasks.json"

    echo ""
    echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}SCENARIO: $scenario${NC}"
    echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
    echo ""

    # Check for tasks.json
    if [ ! -f "$tasks_file" ]; then
        echo -e "${YELLOW}No tasks.json found for $scenario${NC}"
        return 0
    fi

    # Read and execute tasks
    local task_count=$(jq '.tasks | length' "$tasks_file")
    echo "Running $task_count tasks..."
    echo ""

    for i in $(seq 0 $((task_count - 1))); do
        local task_id=$(jq -r ".tasks[$i].id" "$tasks_file")
        local task_desc=$(jq -r ".tasks[$i].description" "$tasks_file")

        echo -e "${YELLOW}Task $task_id:${NC} $task_desc"
        echo ""

        # Send task with auto-approve (uses local models, mDNS discovery)
        "$BINARY" task --yes "$task_desc" 2>&1 || true

        echo ""
        echo "---"
        sleep 2
    done
}

cleanup() {
    echo ""
    echo "Cleaning up..."
    stop_orchestrator
    "$SCRIPT_DIR/mesh.sh" stop 2>/dev/null || true
}

# Main
print_banner

echo ""
echo -e "${YELLOW}TIP: Run 'arkavo ui' in another terminal to visualize agent events${NC}"
echo ""
echo "Scenarios: ${SCENARIOS[*]}"
echo "Delay between scenarios: ${DELAY}s"
echo ""

check_binary
mkdir -p "$SCRIPT_DIR/logs"

# Set up cleanup trap
trap cleanup EXIT

# Start orchestrator
start_orchestrator

# Start mesh
"$SCRIPT_DIR/mesh.sh" start 3
sleep 3

# Run each scenario
for scenario in "${SCENARIOS[@]}"; do
    run_scenario "$scenario"

    echo ""
    echo "Waiting ${DELAY}s before next scenario..."
    echo "(Press Ctrl+C to stop)"
    sleep "$DELAY"
done

echo ""
echo -e "${GREEN}Demo complete!${NC}"
