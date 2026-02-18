#!/bin/bash
# Stop Arkavo bridge agent

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common/cleanup.sh"

PID_FILE="$SCRIPT_DIR/.agent_pids"

echo -e "${CYAN}Stopping Arkavo bridge agent...${NC}"
cleanup_pids "$PID_FILE"
