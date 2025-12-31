#!/bin/bash
# Stop all peer chat agents

cd "$(dirname "$0")"

echo ""
echo "[PEERS ] Stopping Peer Model Chat demonstration..."
echo ""

# Stop Alice
if [ -f .alice.pid ]; then
    PID=$(cat .alice.pid)
    if kill -0 $PID 2>/dev/null; then
        echo "[ALICE ] Stopping Alice (PID: $PID)..."
        kill $PID 2>/dev/null || true
    fi
    rm -f .alice.pid
fi

# Stop Bob
if [ -f .bob.pid ]; then
    PID=$(cat .bob.pid)
    if kill -0 $PID 2>/dev/null; then
        echo "[BOB   ] Stopping Bob (PID: $PID)..."
        kill $PID 2>/dev/null || true
    fi
    rm -f .bob.pid
fi

# Clean up port files
rm -f .alice.port .bob.port

echo ""
echo "[PEERS ] All processes stopped."
echo ""
