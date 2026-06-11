#!/bin/bash
# Stop all shadow-teacher-kit agents.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PID_FILE="$SCRIPT_DIR/.agent_pids"
if [ -f "$PID_FILE" ]; then
    while IFS= read -r pid; do
        kill "$pid" 2>/dev/null || true
    done < "$PID_FILE"
    rm -f "$PID_FILE"
fi
# Kill any straggler agents so they cannot hold ports or stale db handles.
pkill -f "arkavo agent run" 2>/dev/null || true
echo "shadow-teacher-kit agents stopped"
