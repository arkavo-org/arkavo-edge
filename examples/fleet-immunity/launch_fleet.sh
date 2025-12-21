#!/bin/bash
# Launch all 3 rovers for the Fleet Immunity demonstration

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
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ARKAVO="$REPO_ROOT/target/debug/arkavo"

cd "$SCRIPT_DIR"

# Build arkavo if needed
if [ ! -f "$ARKAVO" ]; then
    echo "[BUILD ] Building arkavo CLI..."
    (cd "$REPO_ROOT" && cargo build -p arkavo -q)
fi

# Build environment simulator if needed
if [ ! -f env-simulator/target/debug/fleet-env-simulator ]; then
    echo "[BUILD ] Building environment simulator..."
    (cd env-simulator && cargo build -q)
fi

# Start environment simulator
echo "[ENV   ] Starting warehouse environment on port 8360..."
./env-simulator/target/debug/fleet-env-simulator &
ENV_PID=$!
echo $ENV_PID > .env.pid
sleep 2

# Start rovers
echo ""
echo "[ALPHA ] Starting Rover Alpha on port 8351..."
(cd "$SCRIPT_DIR/rover-alpha" && "$ARKAVO" agent --config AGENTS.md) &
ALPHA_PID=$!
echo $ALPHA_PID > .alpha.pid

sleep 1

echo "[BETA  ] Starting Rover Beta on port 8352..."
(cd "$SCRIPT_DIR/rover-beta" && "$ARKAVO" agent --config AGENTS.md) &
BETA_PID=$!
echo $BETA_PID > .beta.pid

sleep 1

echo "[GAMMA ] Starting Rover Gamma on port 8353..."
(cd "$SCRIPT_DIR/rover-gamma" && "$ARKAVO" agent --config AGENTS.md) &
GAMMA_PID=$!
echo $GAMMA_PID > .gamma.pid

echo ""
echo "[FLEET ] All rovers launched. Waiting for mesh discovery..."
sleep 3

echo ""
echo "[FLEET ] Fleet ready!"
echo ""
echo "  Next steps:"
echo "    1. ./monitor_fleet.sh  - Watch fleet status"
echo "    2. ./inject_hazard.sh  - Inject black ice into Sector 4"
echo "    3. ./stop_fleet.sh     - Stop all rovers"
echo ""
