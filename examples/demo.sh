#!/bin/bash
# Arkavo Demo - Interactive showcase of agent capabilities
#
# Usage:
#   ./demo.sh              # Interactive mode - choose scenarios
#   ./demo.sh all          # Run all scenarios
#   ./demo.sh <scenario>   # Run specific scenario
#   ./demo.sh --list       # List available scenarios

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${BINARY:-$SCRIPT_DIR/../target/debug/arkavo}"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
CYAN='\033[0;36m'
NC='\033[0m'

# Default delay between scenarios
DELAY=10

print_banner() {
    echo -e "${BLUE}"
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║                    Arkavo Demo                                ║"
    echo "║         AI Agent System Showcase                              ║"
    echo "╚═══════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"
}

print_help() {
    echo "Usage: $0 [OPTIONS] [SCENARIO]"
    echo ""
    echo "Options:"
    echo "  --list, -l     List available scenarios"
    echo "  --help, -h     Show this help"
    echo "  all            Run all scenarios"
    echo "  <scenario>     Run specific scenario"
    echo ""
    echo "Without arguments, runs in interactive mode."
    echo ""
    echo "Examples:"
    echo "  $0                      # Interactive mode"
    echo "  $0 --list               # List scenarios"
    echo "  $0 01-hello-world       # Run hello-world"
    echo "  $0 all                  # Run all scenarios"
}

# Find all scenarios with tasks.json
get_scenarios() {
    find "$SCRIPT_DIR" -maxdepth 2 -name "tasks.json" -type f | \
        xargs -I {} dirname {} | \
        xargs -I {} basename {} | \
        sort
}

list_scenarios() {
    echo -e "${CYAN}Available Scenarios${NC}"
    echo "===================="
    echo ""

    for scenario in $(get_scenarios); do
        local tasks_file="$SCRIPT_DIR/$scenario/tasks.json"
        if [ -f "$tasks_file" ]; then
            local desc=$(jq -r '.description // "No description"' "$tasks_file" 2>/dev/null || echo "No description")
            local count=$(jq '.tasks | length' "$tasks_file" 2>/dev/null || echo "?")
            printf "  ${GREEN}%-30s${NC} %s (%s tasks)\n" "$scenario" "$desc" "$count"
        fi
    done
    echo ""
}

check_prerequisites() {
    local errors=0

    echo -e "${CYAN}Checking prerequisites...${NC}"
    echo ""

    # Check binary
    if [ -f "$BINARY" ]; then
        echo -e "  ${GREEN}✓${NC} Arkavo binary found"
    else
        echo -e "  ${RED}✗${NC} Arkavo binary not found at $BINARY"
        echo "    Run: cargo build"
        errors=$((errors + 1))
    fi

    # Check jq
    if command -v jq &> /dev/null; then
        echo -e "  ${GREEN}✓${NC} jq installed"
    else
        echo -e "  ${RED}✗${NC} jq not found (required for parsing tasks.json)"
        echo "    Install: brew install jq (macOS) or apt install jq (Linux)"
        errors=$((errors + 1))
    fi

    # Check for scenarios
    local scenario_count=$(get_scenarios | wc -l | tr -d ' ')
    if [ "$scenario_count" -gt 0 ]; then
        echo -e "  ${GREEN}✓${NC} Found $scenario_count scenarios with tasks.json"
    else
        echo -e "  ${RED}✗${NC} No scenarios found with tasks.json"
        errors=$((errors + 1))
    fi

    echo ""

    if [ $errors -gt 0 ]; then
        echo -e "${RED}Prerequisites check failed. Please fix the issues above.${NC}"
        exit 1
    fi

    echo -e "${GREEN}All prerequisites satisfied!${NC}"
    echo ""
}

start_orchestrator() {
    echo -e "${CYAN}Starting orchestrator...${NC}"
    cd "$SCRIPT_DIR"
    mkdir -p "$SCRIPT_DIR/logs"
    nohup "$BINARY" agent run > "$SCRIPT_DIR/logs/orchestrator.log" 2>&1 &
    echo $! > "$SCRIPT_DIR/.orchestrator_pid"
    sleep 2
    echo -e "  ${GREEN}✓${NC} Orchestrator started (PID: $(cat "$SCRIPT_DIR/.orchestrator_pid"))"
}

stop_orchestrator() {
    if [ -f "$SCRIPT_DIR/.orchestrator_pid" ]; then
        local pid=$(cat "$SCRIPT_DIR/.orchestrator_pid")
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
        echo -e "${YELLOW}No tasks.json found for $scenario - skipping${NC}"
        return 0
    fi

    # Show scenario description
    local desc=$(jq -r '.description // "No description"' "$tasks_file")
    echo -e "${CYAN}Description:${NC} $desc"
    echo ""

    # Read and execute tasks
    local task_count=$(jq '.tasks | length' "$tasks_file")
    echo "Running $task_count tasks..."
    echo ""

    for i in $(seq 0 $((task_count - 1))); do
        local task_id=$(jq -r ".tasks[$i].id" "$tasks_file")
        local task_desc=$(jq -r ".tasks[$i].description" "$tasks_file")

        echo -e "${YELLOW}Task $((i + 1))/$task_count [$task_id]:${NC}"
        echo "  $task_desc"
        echo ""

        # Send task with auto-approve (uses local models, mDNS discovery)
        "$BINARY" task --yes "$task_desc" 2>&1 || true

        echo ""
        echo "---"

        # Brief pause between tasks
        if [ $i -lt $((task_count - 1)) ]; then
            sleep 2
        fi
    done

    echo ""
    echo -e "${GREEN}Scenario '$scenario' complete!${NC}"
}

interactive_mode() {
    echo -e "${CYAN}Interactive Mode${NC}"
    echo "================"
    echo ""
    echo "Select scenarios to run:"
    echo ""

    local scenarios=($(get_scenarios))
    local i=1

    for scenario in "${scenarios[@]}"; do
        printf "  %d) %s\n" "$i" "$scenario"
        i=$((i + 1))
    done

    echo ""
    printf "  a) Run all scenarios\n"
    printf "  q) Quit\n"
    echo ""

    read -p "Enter choice(s) separated by spaces [1-${#scenarios[@]}/a/q]: " choices

    if [ "$choices" = "q" ]; then
        echo "Goodbye!"
        exit 0
    fi

    if [ "$choices" = "a" ]; then
        run_all_scenarios
        return
    fi

    # Parse and run selected scenarios
    for choice in $choices; do
        if [[ "$choice" =~ ^[0-9]+$ ]] && [ "$choice" -ge 1 ] && [ "$choice" -le "${#scenarios[@]}" ]; then
            local idx=$((choice - 1))
            run_scenario "${scenarios[$idx]}"
            sleep 2
        else
            echo -e "${YELLOW}Invalid choice: $choice${NC}"
        fi
    done
}

run_all_scenarios() {
    local scenarios=($(get_scenarios))
    local total=${#scenarios[@]}
    local current=1

    echo ""
    echo -e "${CYAN}Running all $total scenarios...${NC}"
    echo ""

    for scenario in "${scenarios[@]}"; do
        echo -e "${BLUE}[$current/$total]${NC}"
        run_scenario "$scenario"

        if [ $current -lt $total ]; then
            echo ""
            echo "Waiting ${DELAY}s before next scenario... (Ctrl+C to stop)"
            sleep "$DELAY"
        fi

        current=$((current + 1))
    done
}

cleanup() {
    echo ""
    echo -e "${CYAN}Cleaning up...${NC}"
    stop_orchestrator
    "$SCRIPT_DIR/mesh.sh" stop 2>/dev/null || true
    echo -e "${GREEN}Done!${NC}"
}

# Main
case "${1:-}" in
    --help|-h)
        print_help
        exit 0
        ;;
    --list|-l)
        list_scenarios
        exit 0
        ;;
esac

print_banner

echo -e "${YELLOW}TIP: Run 'arkavo ui' in another terminal to visualize agent events${NC}"
echo ""

check_prerequisites

# Set up cleanup trap
trap cleanup EXIT

# Start infrastructure
start_orchestrator
echo ""
"$SCRIPT_DIR/mesh.sh" start 3
sleep 3
echo ""

# Run based on argument
case "${1:-}" in
    "")
        interactive_mode
        ;;
    all)
        run_all_scenarios
        ;;
    *)
        if [ -d "$SCRIPT_DIR/$1" ]; then
            run_scenario "$1"
        else
            echo -e "${RED}Unknown scenario: $1${NC}"
            echo ""
            list_scenarios
            exit 1
        fi
        ;;
esac

echo ""
echo -e "${GREEN}Demo complete!${NC}"
