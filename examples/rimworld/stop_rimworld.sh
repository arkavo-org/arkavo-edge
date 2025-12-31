#!/bin/bash
# Stop RimWorld Survival Swarm agents

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIDS_FILE="$SCRIPT_DIR/.pids"

echo "Stopping RimWorld Survival Swarm..."

if [ -f "$PIDS_FILE" ]; then
    while IFS=':' read -r pid name; do
        if kill -0 "$pid" 2>/dev/null; then
            echo "Stopping $name (PID: $pid)"
            kill "$pid" 2>/dev/null || true
        fi
    done < "$PIDS_FILE"
    rm -f "$PIDS_FILE"
fi

# Clean up any remaining arkavo processes
pkill -f "arkavo agent" 2>/dev/null || true

echo "All agents stopped."
