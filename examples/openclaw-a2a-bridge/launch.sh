#!/bin/bash
# Launch Arkavo bridge agent for OpenClaw A2A demo

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common/check_prerequisites.sh"
source "$SCRIPT_DIR/../common/run_agent.sh"

BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"
PID_FILE="$SCRIPT_DIR/.agent_pids"
LOG_DIR="$SCRIPT_DIR/logs"
PORT=8360

echo -e "${CYAN}OpenClaw A2A Bridge — Arkavo Agent${NC}"
echo "===================================="
echo ""

# Check binary
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}Error: Arkavo binary not found${NC}"
    echo "Build with: cargo build -p arkavo --features kas"
    exit 1
fi

# Check ports
if ! check_ports "$PORT"; then
    exit 1
fi

# Clean stale PIDs
rm -f "$PID_FILE"
mkdir -p "$LOG_DIR"

echo -e "${GREEN}Starting Arkavo bridge agent...${NC}"
echo ""
echo -e "  Endpoint : http://localhost:${PORT}"
echo -e "  Agent Card: http://localhost:${PORT}/.well-known/agent.json"
echo -e "  KAS      : enabled (key: bridge-demo-key-1, ec:secp256r1)"
echo -e "  Preflight: 2 policies active (block_pii, block_shell_commands)"
echo -e "  Budget   : \$1.00 session cap"
echo -e "  Model    : ministral-3b (local)"
echo ""

# Start agent
nohup "$BINARY" agent run --port "$PORT" --verbose >"$LOG_DIR/agent.log" 2>&1 &
AGENT_PID=$!
echo "$AGENT_PID arkavo-bridge-agent" >> "$PID_FILE"

# Wait for agent to become healthy
if wait_for_health "http://localhost:${PORT}" "Arkavo bridge agent" 30 1; then
    echo -e "${GREEN}Agent started (PID: ${AGENT_PID})${NC}"
else
    echo -e "${RED}Agent failed to start. Check logs:${NC}"
    tail -20 "$LOG_DIR/agent.log" 2>/dev/null || true
    exit 1
fi
