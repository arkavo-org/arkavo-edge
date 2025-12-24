#!/bin/bash
# Stop all rovers and fleet environment server

cd "$(dirname "$0")"

echo ""
echo "[FLEET ] Stopping Fleet Immunity demonstration..."
echo ""

# Stop rovers
for name in alpha beta gamma; do
    pidfile=".${name}.pid"
    if [ -f "$pidfile" ]; then
        PID=$(cat "$pidfile")
        NAME=$(echo "$name" | tr '[:lower:]' '[:upper:]')
        if kill -0 $PID 2>/dev/null; then
            printf "[%-6s] Stopping Rover %s (PID: %s)...\n" "$NAME" "$NAME" "$PID"
            kill $PID 2>/dev/null || true
        fi
        rm -f "$pidfile"
    fi
done

# Stop fleet environment server
if [ -f .fleet-env.pid ]; then
    PID=$(cat .fleet-env.pid)
    if kill -0 $PID 2>/dev/null; then
        echo "[ENV   ] Stopping Fleet Environment server (PID: $PID)..."
        kill $PID 2>/dev/null || true
    fi
    rm -f .fleet-env.pid
fi

echo ""
echo "[FLEET ] All processes stopped."
echo ""
