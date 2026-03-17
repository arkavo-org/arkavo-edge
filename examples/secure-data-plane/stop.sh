#!/bin/bash
# Stop secure data plane agents

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/../common/cleanup.sh"

PID_FILE="${SCRIPT_DIR}/logs/pids"
stop_agents "$PID_FILE"
