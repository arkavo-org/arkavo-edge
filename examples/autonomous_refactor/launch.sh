#!/bin/bash
# Launch the Refactor Mesh for the Autonomous Refactor demonstration
#
# Architecture:
# - One analyzer agent (identifies errors, coordinates work)
# - Three fixer agents (each handles one service)

set -e

echo ""
echo "  ╔═══════════════════════════════════════════════════════════════╗"
echo "  ║                                                               ║"
echo "  ║     ARKAVO EDGE - AUTONOMOUS REFACTOR DEMONSTRATION           ║"
echo "  ║                                                               ║"
echo "  ║         Multi-Agent Mesh Fixes Breaking API Change            ║"
echo "  ║                                                               ║"
echo "  ╚═══════════════════════════════════════════════════════════════╝"
echo ""

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ARKAVO_BIN="${ARKAVO_BIN:-${SCRIPT_DIR}/../../target/debug/arkavo}"

cd "$SCRIPT_DIR"

# Check prerequisites
if [ ! -f "$ARKAVO_BIN" ]; then
    echo "[BUILD ] Building Arkavo..."
    (cd ../.. && cargo build -q -p arkavo)
fi

# Ensure demo_workspace exists
if [ ! -d "demo_workspace" ]; then
    echo "[SETUP ] Creating demo workspace..."
    ./run_demo.sh
fi

# Create logs directory
mkdir -p logs pids

KIT="$SCRIPT_DIR/autonomous-refactor.swarmkit.yaml"

echo ""
echo "[MESH  ] Starting refactor agents..."
echo ""

# Start analyzer agent
"$ARKAVO_BIN" agent -c "$KIT" -n refactor-analyzer > "$SCRIPT_DIR/logs/analyzer.log" 2>&1 &
ANALYZER_PID=$!
echo $ANALYZER_PID > pids/analyzer.pid
echo "[ANALYZER] Started Refactor Analyzer (PID: $ANALYZER_PID)"

sleep 1

# Start fixer agents
"$ARKAVO_BIN" agent -c "$KIT" -n fixer-alpha > "$SCRIPT_DIR/logs/fixer-alpha.log" 2>&1 &
ALPHA_PID=$!
echo $ALPHA_PID > pids/fixer-alpha.pid
echo "[ALPHA   ] Started Fixer Alpha - service_a (PID: $ALPHA_PID)"

sleep 1

"$ARKAVO_BIN" agent -c "$KIT" -n fixer-beta > "$SCRIPT_DIR/logs/fixer-beta.log" 2>&1 &
BETA_PID=$!
echo $BETA_PID > pids/fixer-beta.pid
echo "[BETA    ] Started Fixer Beta - service_b (PID: $BETA_PID)"

sleep 1

"$ARKAVO_BIN" agent -c "$KIT" -n fixer-gamma > "$SCRIPT_DIR/logs/fixer-gamma.log" 2>&1 &
GAMMA_PID=$!
echo $GAMMA_PID > pids/fixer-gamma.pid
echo "[GAMMA   ] Started Fixer Gamma - service_c (PID: $GAMMA_PID)"

echo ""
echo "[MESH  ] Waiting for mesh discovery..."
sleep 3

echo ""
echo "[MESH  ] Refactor mesh ready!"
echo ""
echo "  Next steps:"
echo "    1. Check logs: tail -f logs/*.log"
echo "    2. Submit task: arkavo task run --prompt 'Fix all build errors in demo_workspace'"
echo "    3. Stop mesh: ./stop.sh"
echo ""
