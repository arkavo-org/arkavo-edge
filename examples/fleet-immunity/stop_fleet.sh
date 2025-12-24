#!/bin/bash
# Stop all rovers

cd "$(dirname "$0")"

echo ""
echo "[FLEET ] Stopping Fleet Immunity demonstration..."
echo ""

# Stop rovers
if [ -f .alpha.pid ]; then
    PID=$(cat .alpha.pid)
    if kill -0 $PID 2>/dev/null; then
        echo "[ALPHA ] Stopping Rover Alpha (PID: $PID)..."
        kill $PID 2>/dev/null || true
    fi
    rm -f .alpha.pid
fi

if [ -f .beta.pid ]; then
    PID=$(cat .beta.pid)
    if kill -0 $PID 2>/dev/null; then
        echo "[BETA  ] Stopping Rover Beta (PID: $PID)..."
        kill $PID 2>/dev/null || true
    fi
    rm -f .beta.pid
fi

if [ -f .gamma.pid ]; then
    PID=$(cat .gamma.pid)
    if kill -0 $PID 2>/dev/null; then
        echo "[GAMMA ] Stopping Rover Gamma (PID: $PID)..."
        kill $PID 2>/dev/null || true
    fi
    rm -f .gamma.pid
fi

echo ""
echo "[FLEET ] All processes stopped."
echo ""
