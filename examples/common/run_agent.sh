#!/bin/bash
# run_agent.sh - Shared agent launcher for examples
#
# Usage: source this file from your launch script
#   source "$(dirname "$0")/../common/run_agent.sh"
#   start_agent "$BINARY" "$CONFIG_PATH" "$LOG_FILE" "$PID_FILE"

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Start a single agent
start_agent() {
    local binary="$1"
    local config="$2"
    local log_file="$3"
    local pid_file="$4"
    local name="${5:-Agent}"

    if [[ ! -f "$config" ]]; then
        echo -e "${RED}Error: Config not found: $config${NC}"
        return 1
    fi

    # Ensure log directory exists
    mkdir -p "$(dirname "$log_file")"

    echo -e "${BLUE}Starting $name...${NC}"

    # Start agent in background
    nohup "$binary" agent --config "$config" >"$log_file" 2>&1 &
    local pid=$!

    # Record PID
    echo "$pid $name" >>"$pid_file"

    # Brief wait to check if process started
    sleep 0.5
    if ! ps -p "$pid" >/dev/null 2>&1; then
        echo -e "${RED}Error: $name failed to start${NC}"
        echo "Last log output:"
        tail -10 "$log_file" 2>/dev/null || true
        return 1
    fi

    echo -e "${GREEN}✓${NC} $name started (PID $pid)"
}

# Start multiple agents from a directory of AGENTS.md files
start_agents_from_dir() {
    local binary="$1"
    local agents_dir="$2"
    local log_dir="$3"
    local pid_file="$4"

    for config in "$agents_dir"/*/AGENTS.md; do
        [[ -f "$config" ]] || continue

        local name
        name=$(basename "$(dirname "$config")")
        local log_file="$log_dir/$name.log"

        start_agent "$binary" "$config" "$log_file" "$pid_file" "$name"
    done
}

# Print status of running agents
print_agent_status() {
    local pid_file="$1"

    if [[ ! -f "$pid_file" ]]; then
        echo -e "${YELLOW}No agents running${NC}"
        return 0
    fi

    echo -e "${BLUE}Agent Status:${NC}"
    while read -r pid name; do
        [[ -z "$pid" ]] && continue
        if ps -p "$pid" >/dev/null 2>&1; then
            echo -e "  ${GREEN}●${NC} $name (PID $pid)"
        else
            echo -e "  ${RED}○${NC} $name (PID $pid) - NOT RUNNING"
        fi
    done <"$pid_file"
}
