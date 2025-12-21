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

cd "$SCRIPT_DIR"

# Build environment simulator if needed
if [ ! -f env-simulator/target/debug/fleet-env-simulator ]; then
    echo "[BUILD ] Building environment simulator..."
    (cd env-simulator && cargo build -q)
fi

# Build rover simulator if needed
if [ ! -f rover-simulator/target/debug/rover-simulator ]; then
    echo "[BUILD ] Building rover simulator..."
    (cd rover-simulator && cargo build -q)
fi

ROVER_SIM="$SCRIPT_DIR/rover-simulator/target/debug/rover-simulator"

# Start environment simulator
echo "[ENV   ] Starting warehouse environment on port 8360..."
./env-simulator/target/debug/fleet-env-simulator &
ENV_PID=$!
echo $ENV_PID > .env.pid
sleep 2

# Define peer URLs for each rover
ALPHA_URL="http://localhost:8351"
BETA_URL="http://localhost:8352"
GAMMA_URL="http://localhost:8353"

# Start rovers with their routes
echo ""
"$ROVER_SIM" --name alpha --port 8351 --route "1,2,4,3" --peers "$BETA_URL,$GAMMA_URL" &
ALPHA_PID=$!
echo $ALPHA_PID > .alpha.pid

sleep 1

"$ROVER_SIM" --name beta --port 8352 --route "2,3,4,1" --peers "$ALPHA_URL,$GAMMA_URL" &
BETA_PID=$!
echo $BETA_PID > .beta.pid

sleep 1

"$ROVER_SIM" --name gamma --port 8353 --route "3,1,2,4" --peers "$ALPHA_URL,$BETA_URL" &
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
