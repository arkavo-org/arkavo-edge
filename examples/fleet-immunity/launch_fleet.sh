#!/bin/bash
# Launch all 3 rovers for the Fleet Immunity demonstration
#
# This example demonstrates:
# - Multi-agent coordination via A2A protocol
# - Collective learning from crashes
# - Policy synthesis and consensus

set -e

echo ""
echo "  ╔═══════════════════════════════════════════════════════════════╗"
echo "  ║                                                               ║"
echo "  ║     ARKAVO EDGE - FLEET IMMUNITY DEMONSTRATION                ║"
echo "  ║                                                               ║"
echo "  ║         Autonomous Rovers Learn from Crashes                  ║"
echo "  ║                                                               ║"
echo "  ╚═══════════════════════════════════════════════════════════════╝"
echo ""

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ARKAVO_BIN="${SCRIPT_DIR}/../../target/debug/arkavo"

cd "$SCRIPT_DIR"

# Check prerequisites
if [ ! -f "$ARKAVO_BIN" ]; then
    echo "[BUILD ] Building Arkavo..."
    (cd ../.. && cargo build -q -p arkavo)
fi

# Build the MCP fleet-env crate
if [ ! -f "mcp-fleet-env/target/debug/libarkavo_mcp_fleet_env.rlib" ]; then
    echo "[BUILD ] Building fleet environment MCP tools..."
    (cd mcp-fleet-env && cargo build -q)
fi

# Create logs directory
mkdir -p logs rover-alpha/logs rover-beta/logs rover-gamma/logs

echo ""
echo "[FLEET ] Starting rovers..."
echo ""

# Start rovers as arkavo agents
"$ARKAVO_BIN" agent --config "$SCRIPT_DIR/rover-alpha/AGENTS.md" > logs/alpha.log 2>&1 &
ALPHA_PID=$!
echo $ALPHA_PID > .alpha.pid
echo "[ALPHA ] Started Rover Alpha on port 8351 (PID: $ALPHA_PID)"

sleep 1

"$ARKAVO_BIN" agent --config "$SCRIPT_DIR/rover-beta/AGENTS.md" > logs/beta.log 2>&1 &
BETA_PID=$!
echo $BETA_PID > .beta.pid
echo "[BETA  ] Started Rover Beta on port 8352 (PID: $BETA_PID)"

sleep 1

"$ARKAVO_BIN" agent --config "$SCRIPT_DIR/rover-gamma/AGENTS.md" > logs/gamma.log 2>&1 &
GAMMA_PID=$!
echo $GAMMA_PID > .gamma.pid
echo "[GAMMA ] Started Rover Gamma on port 8353 (PID: $GAMMA_PID)"

echo ""
echo "[FLEET ] Waiting for mesh discovery..."
sleep 3

echo ""
echo "[FLEET ] Fleet ready!"
echo ""
echo "  Next steps:"
echo "    1. ./monitor_fleet.sh  - Watch fleet status"
echo "    2. ./inject_hazard.sh  - Inject black ice into Sector 4"
echo "    3. ./stop_fleet.sh     - Stop all rovers"
echo ""
