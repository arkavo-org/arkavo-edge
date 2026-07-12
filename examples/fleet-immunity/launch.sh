#!/bin/bash
# Launch all 3 rovers for the Fleet Immunity demonstration
#
# Architecture:
# - One shared fleet-env HTTP server (port 8360)
# - Three rover agents, each with MCP proxy to shared server

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
ARKAVO_BIN="${ARKAVO_BIN:-${SCRIPT_DIR}/../../target/debug/arkavo}"
FLEET_ENV_BIN="${SCRIPT_DIR}/mcp-fleet-env/target/debug/arkavo-mcp-fleet-env"

cd "$SCRIPT_DIR"

# Check prerequisites
if [ ! -f "$ARKAVO_BIN" ]; then
    echo "[BUILD ] Building Arkavo..."
    (cd ../.. && cargo build -q -p arkavo)
fi

# Build the fleet-env server
if [ ! -f "$FLEET_ENV_BIN" ]; then
    echo "[BUILD ] Building fleet environment server..."
    (cd mcp-fleet-env && cargo build -q)
fi

# Create logs directory
mkdir -p logs rover-alpha/logs rover-beta/logs rover-gamma/logs

# Start the shared fleet environment server
echo ""
echo "[ENV   ] Starting shared Fleet Environment server on port 8360..."
"$FLEET_ENV_BIN" --serve --port 8360 --sector-count 4 > logs/fleet-env.log 2>&1 &
ENV_PID=$!
echo $ENV_PID > .fleet-env.pid
echo "[ENV   ] Fleet Environment server started (PID: $ENV_PID)"

# Wait for server to be ready
sleep 1
if ! curl -s http://localhost:8360/health > /dev/null 2>&1; then
    echo "[ERROR ] Fleet Environment server failed to start"
    cat logs/fleet-env.log
    exit 1
fi
echo "[ENV   ] Fleet Environment server ready"

echo ""
echo "[FLEET ] Starting rovers..."
echo ""

# Start rovers as arkavo agents, each as one role of the shared fleet-immunity
# kit. RUST_LOG enables tracing output for gossip learning visibility.
export RUST_LOG="${RUST_LOG:-info}"

KIT="$SCRIPT_DIR/fleet-immunity.swarmkit.yaml"

(cd "$SCRIPT_DIR/rover-alpha" && "$ARKAVO_BIN" agent -c "$KIT" -n rover-alpha > "$SCRIPT_DIR/logs/alpha.log" 2>&1) &
ALPHA_PID=$!
echo $ALPHA_PID > .alpha.pid
echo "[ALPHA ] Started Rover Alpha (PID: $ALPHA_PID)"

sleep 1

(cd "$SCRIPT_DIR/rover-beta" && "$ARKAVO_BIN" agent -c "$KIT" -n rover-beta > "$SCRIPT_DIR/logs/beta.log" 2>&1) &
BETA_PID=$!
echo $BETA_PID > .beta.pid
echo "[BETA  ] Started Rover Beta (PID: $BETA_PID)"

sleep 1

(cd "$SCRIPT_DIR/rover-gamma" && "$ARKAVO_BIN" agent -c "$KIT" -n rover-gamma > "$SCRIPT_DIR/logs/gamma.log" 2>&1) &
GAMMA_PID=$!
echo $GAMMA_PID > .gamma.pid
echo "[GAMMA ] Started Rover Gamma (PID: $GAMMA_PID)"

echo ""
echo "[FLEET ] Waiting for mesh discovery..."
sleep 3

echo ""
echo "[FLEET ] Fleet ready!"
echo ""
echo "  Next steps:"
echo "    1. ./monitor_fleet.sh  - Watch fleet status"
echo "    2. ./inject_hazard.sh  - Inject black ice into Sector 4"
echo "    3. ./stop.sh           - Stop all rovers"
echo ""
