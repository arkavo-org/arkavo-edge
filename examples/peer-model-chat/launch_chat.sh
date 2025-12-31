#!/bin/bash
# Launch peer-model-chat demonstration
#
# Two peer agents chat about their model capabilities

set -e

echo ""
echo "  ╔═══════════════════════════════════════════════════════════════╗"
echo "  ║                                                               ║"
echo "  ║     ARKAVO EDGE - PEER MODEL CHAT DEMONSTRATION              ║"
echo "  ║                                                               ║"
echo "  ║         Two Agents Collaborate Using Different Models        ║"
echo "  ║                                                               ║"
echo "  ╚═══════════════════════════════════════════════════════════════╝"
echo ""

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ARKAVO_BIN="${ARKAVO_BIN:-${SCRIPT_DIR}/../../target/debug/arkavo}"

# Configured ports from AGENTS.md
ALICE_PORT=8370
BOB_PORT=8371

cd "$SCRIPT_DIR"

# Check prerequisites
if [ ! -f "$ARKAVO_BIN" ]; then
    echo "[BUILD ] Building Arkavo..."
    (cd ../.. && cargo build -q -p arkavo)
fi

# Create logs directories
mkdir -p logs alice/logs bob/logs

# Save ports to files for inject_question.sh
echo "$ALICE_PORT" > .alice.port
echo "$BOB_PORT" > .bob.port

echo ""
echo "[PEERS ] Starting peer agents..."
echo ""

# Set logging level
export RUST_LOG="${RUST_LOG:-info}"

# Function to check agent health via RPC
check_health() {
    local port=$1
    curl -s -X POST "http://localhost:$port/rpc" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"rpc.discover","params":{},"id":1}' \
        2>/dev/null | grep -q '"result"'
}

# Function to wait for agent to be ready
wait_for_agent() {
    local name=$1
    local port=$2
    local max_wait=30
    local waited=0

    while [ $waited -lt $max_wait ]; do
        if check_health "$port"; then
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    return 1
}

# Start Alice (Qwen3 model - fast reasoning)
(cd "$SCRIPT_DIR/alice" && "$ARKAVO_BIN" agent --config AGENTS.md > "$SCRIPT_DIR/logs/alice.log" 2>&1) &
ALICE_PID=$!
echo $ALICE_PID > .alice.pid
echo "[ALICE ] Started Alice (Qwen3-0.6B) (PID: $ALICE_PID)"

sleep 2

# Start Bob (Ministral-3B model - structured output)
(cd "$SCRIPT_DIR/bob" && "$ARKAVO_BIN" agent --config AGENTS.md > "$SCRIPT_DIR/logs/bob.log" 2>&1) &
BOB_PID=$!
echo $BOB_PID > .bob.pid
echo "[BOB   ] Started Bob (Ministral-3B) (PID: $BOB_PID)"

echo ""
echo "[PEERS ] Waiting for agents to start..."

# Wait for Alice
if wait_for_agent "Alice" "$ALICE_PORT"; then
    echo "[ALICE ] Listening on port $ALICE_PORT - Health check passed"
else
    echo "[ERROR ] Alice failed to start on port $ALICE_PORT"
    echo ""
    echo "Last 30 lines of alice.log:"
    tail -30 logs/alice.log
    exit 1
fi

# Wait for Bob
if wait_for_agent "Bob" "$BOB_PORT"; then
    echo "[BOB   ] Listening on port $BOB_PORT - Health check passed"
else
    echo "[ERROR ] Bob failed to start on port $BOB_PORT"
    echo ""
    echo "Last 30 lines of bob.log:"
    tail -30 logs/bob.log
    exit 1
fi

echo ""
echo "[PEERS ] Waiting for mDNS discovery..."
sleep 3

echo ""
echo "[PEERS ] Peer chat system ready!"
echo ""
echo "  Agent Endpoints:"
echo "    Alice (Qwen3):      http://localhost:$ALICE_PORT"
echo "    Bob (Ministral-3):  http://localhost:$BOB_PORT"
echo ""
echo "  Next steps:"
echo "    1. ./inject_question.sh       - Ask a question that triggers peer collaboration"
echo "    2. tail -f logs/*.log         - Monitor agent communication"
echo "    3. ./stop_chat.sh             - Stop all agents"
echo ""
