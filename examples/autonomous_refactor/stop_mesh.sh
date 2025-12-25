#!/bin/bash
# Stop all agents in the Refactor Mesh

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo ""
echo "[MESH  ] Stopping refactor agents..."

for pidfile in pids/*.pid; do
    if [ -f "$pidfile" ]; then
        PID=$(cat "$pidfile")
        NAME=$(basename "$pidfile" .pid)
        if kill -0 "$PID" 2>/dev/null; then
            kill "$PID" 2>/dev/null
            echo "[STOP  ] Stopped $NAME (PID: $PID)"
        else
            echo "[SKIP  ] $NAME already stopped"
        fi
        rm -f "$pidfile"
    fi
done

echo ""
echo "[MESH  ] All agents stopped."
echo ""
