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

cd "$(dirname "$0")"

# Check if arkavo is available
if ! command -v arkavo &> /dev/null; then
    echo "[ERROR] arkavo CLI not found. Please build with: cargo build"
    exit 1
fi

# Start environment simulator
echo "[ENV   ] Starting warehouse environment on port 8360..."
arkavo environment --config environment/warehouse.yaml &
ENV_PID=$!
echo $ENV_PID > .env.pid
sleep 2

# Start rovers
echo ""
echo "[ALPHA ] Starting Rover Alpha on port 8351..."
cd rover-alpha && arkavo agent --config AGENTS.md &
ALPHA_PID=$!
echo $ALPHA_PID > ../.alpha.pid
cd ..

sleep 1

echo "[BETA  ] Starting Rover Beta on port 8352..."
cd rover-beta && arkavo agent --config AGENTS.md &
BETA_PID=$!
echo $BETA_PID > ../.beta.pid
cd ..

sleep 1

echo "[GAMMA ] Starting Rover Gamma on port 8353..."
cd rover-gamma && arkavo agent --config AGENTS.md &
GAMMA_PID=$!
echo $GAMMA_PID > ../.gamma.pid
cd ..

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
