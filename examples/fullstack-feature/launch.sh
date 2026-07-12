#!/bin/bash
# Fullstack Feature Development - Mesh Example
#
# Demonstrates a 3-agent mesh for fullstack development:
# - Frontend agent: React components, forms, API clients
# - Backend agent: REST APIs, database, business logic
# - Orchestrator: Coordinates both for feature implementation
#
# Usage:
#   ./launch.sh                    # Use the kit's configured model
#   ./launch.sh stop               # Stop all agents
#
# Model selection now lives in fullstack-feature.swarmkit.yaml
# (agent_provisioning.model per role) rather than a CLI flag — `arkavo
# agent` has no model-override flag. Edit the kit file to change models.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${BINARY:-$SCRIPT_DIR/../../target/debug/arkavo}"
KIT="$SCRIPT_DIR/fullstack-feature.swarmkit.yaml"
PID_FILE="$SCRIPT_DIR/.agent_pids"
LOG_DIR="$SCRIPT_DIR/logs"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        stop)
            echo -e "${CYAN}Stopping agents...${NC}"
            if [ -f "$PID_FILE" ]; then
                while read -r pid name; do
                    [ -z "$pid" ] && continue
                    kill "$pid" 2>/dev/null && echo -e "  ${GREEN}✓${NC} Stopped $name (PID $pid)" || true
                done < "$PID_FILE"
                rm -f "$PID_FILE"
            fi
            echo -e "${GREEN}Done!${NC}"
            exit 0
            ;;
        *)
            shift
            ;;
    esac
done

print_banner() {
    echo -e "${BLUE}"
    echo "================================================================"
    echo "     Fullstack Feature Development Mesh"
    echo "================================================================"
    echo -e "${NC}"
    echo ""
    echo "This example demonstrates:"
    echo "  - Frontend agent for UI components"
    echo "  - Backend agent for REST APIs"
    echo "  - Orchestrator to coordinate both"
    echo ""
}

check_prerequisites() {
    echo -e "${CYAN}Checking prerequisites...${NC}"

    if [ ! -f "$BINARY" ]; then
        echo -e "${RED}Error: Arkavo binary not found at $BINARY${NC}"
        echo "Run: cargo build"
        exit 1
    fi
    echo -e "  ${GREEN}✓${NC} Arkavo binary found"

    if ! command -v jq &> /dev/null; then
        echo -e "${RED}Error: jq not found${NC}"
        exit 1
    fi
    echo -e "  ${GREEN}✓${NC} jq installed"

    echo ""
}

start_agent() {
    local name="$1"
    local role="$2"
    local port="$3"
    local log_file="$LOG_DIR/$name.log"

    mkdir -p "$LOG_DIR"

    echo -e "${BLUE}Starting $name on port $port...${NC}"

    nohup "$BINARY" agent -c "$KIT" -n "$role" -p "$port" > "$log_file" 2>&1 &
    local pid=$!

    echo "$pid $name" >> "$PID_FILE"

    sleep 1
    if ps -p "$pid" > /dev/null 2>&1; then
        echo -e "  ${GREEN}✓${NC} $name started (PID $pid)"
    else
        echo -e "  ${RED}✗${NC} $name failed to start"
        tail -5 "$log_file"
        return 1
    fi
}

run_tasks() {
    local tasks_file="$SCRIPT_DIR/tasks.json"
    local task_count
    task_count=$(jq '.tasks | length' "$tasks_file")

    echo ""
    echo -e "${CYAN}Running $task_count fullstack feature tasks...${NC}"
    echo ""

    for i in $(seq 0 $((task_count - 1))); do
        local task_id
        local task_desc
        task_id=$(jq -r ".tasks[$i].id" "$tasks_file")
        task_desc=$(jq -r ".tasks[$i].description" "$tasks_file")

        echo -e "${YELLOW}Task [$task_id]:${NC}"
        echo "  $task_desc"
        echo ""

        # Task uses mesh agents (which have model configured) or falls back to local
        "$BINARY" task --yes "$task_desc" 2>&1 || true

        echo ""
        echo "---"
        echo ""

        # Pause between tasks
        if [ $i -lt $((task_count - 1)) ]; then
            sleep 2
        fi
    done
}

cleanup() {
    echo ""
    echo -e "${CYAN}Cleaning up...${NC}"
    if [ -f "$PID_FILE" ]; then
        while read -r pid name; do
            [ -z "$pid" ] && continue
            kill "$pid" 2>/dev/null || true
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi
    echo -e "${GREEN}Done!${NC}"
}

# Main
print_banner

check_prerequisites

# Clean up any existing agents
rm -f "$PID_FILE"

# Start the mesh agents (roles of fullstack-feature.swarmkit.yaml, each on
# its historical listen port)
echo -e "${CYAN}Starting mesh agents...${NC}"
start_agent "frontend-agent" "frontend-agent" 8370
start_agent "backend-agent" "backend-agent" 8380
start_agent "orchestrator" "orchestrator-agent" 8390
echo ""

# Give agents time to register on mesh
echo -e "${CYAN}Waiting for mesh discovery...${NC}"
sleep 3
echo ""

# Set up cleanup trap
trap cleanup EXIT

# Run the feature tasks
run_tasks

echo ""
echo -e "${GREEN}Fullstack feature development complete!${NC}"
