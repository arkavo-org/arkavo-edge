#!/bin/bash
# cleanup.sh - Shared cleanup utilities for examples
#
# Usage: source this file from your stop script
#   source "$(dirname "$0")/../common/cleanup.sh"
#   cleanup_pids "$PID_FILE"

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Kill process by PID with graceful fallback
kill_pid() {
    local pid="$1"
    local name="${2:-Process}"

    if ! ps -p "$pid" >/dev/null 2>&1; then
        return 0
    fi

    echo -n "Stopping $name (PID $pid)..."

    # Try graceful shutdown first
    kill "$pid" 2>/dev/null || true
    sleep 1

    # Force kill if still running
    if ps -p "$pid" >/dev/null 2>&1; then
        kill -9 "$pid" 2>/dev/null || true
        echo -e " ${YELLOW}forced${NC}"
    else
        echo -e " ${GREEN}OK${NC}"
    fi
}

# Cleanup all PIDs from a file
cleanup_pids() {
    local pid_file="$1"

    if [[ ! -f "$pid_file" ]]; then
        echo -e "${YELLOW}No PID file found${NC}"
        return 0
    fi

    while read -r pid name; do
        [[ -z "$pid" ]] && continue
        kill_pid "$pid" "${name:-Agent}"
    done <"$pid_file"

    rm -f "$pid_file"
    echo -e "${GREEN}✓${NC} Cleanup complete"
}

# Kill processes on specific ports
cleanup_ports() {
    local ports=("$@")

    for port in "${ports[@]}"; do
        local pids
        pids=$(lsof -ti ":$port" 2>/dev/null || true)
        if [[ -n "$pids" ]]; then
            echo "Killing processes on port $port..."
            echo "$pids" | xargs kill -9 2>/dev/null || true
        fi
    done
}

# Cleanup logs directory (keep structure, remove files)
cleanup_logs() {
    local log_dir="$1"

    if [[ -d "$log_dir" ]]; then
        find "$log_dir" -type f -name "*.log" -delete
        echo -e "${GREEN}✓${NC} Logs cleaned"
    fi
}
