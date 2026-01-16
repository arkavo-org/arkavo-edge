#!/bin/bash
# Stop HYPERforum AI Council mesh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"

echo "Stopping HYPERforum AI Council..."

AGENTS=(conductor router critic synthesis critical-analyst researcher synthesizer devils-advocate facilitator)

for agent in "${AGENTS[@]}"; do
    PID_FILE="$LOG_DIR/$agent.pid"
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            echo "Stopping $agent (PID: $PID)..."
            kill "$PID" 2>/dev/null || true
        fi
        rm -f "$PID_FILE"
    fi
done

echo "AI Council mesh stopped."
