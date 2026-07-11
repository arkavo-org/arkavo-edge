#!/bin/bash
# Launch HYPERforum AI Council mesh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARKAVO_BIN="${ARKAVO_BIN:-$(cd "$SCRIPT_DIR/../.." && pwd)/target/debug/arkavo}"
LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR"

# Verify arkavo binary exists
if [[ ! -x "$ARKAVO_BIN" ]]; then
    echo "Error: arkavo binary not found at $ARKAVO_BIN"
    echo "Build with: cargo build"
    exit 1
fi

echo "Launching HYPERforum AI Council..."
echo "Using arkavo: $ARKAVO_BIN"

KIT="$SCRIPT_DIR/hyperforum-council.swarmkit.yaml"

# Launch infrastructure agents
echo "Starting Conductor on port 8501..."
"$ARKAVO_BIN" agent -c "$KIT" -n "council-conductor" -p 8501 > "$LOG_DIR/conductor.log" 2>&1 &
echo $! > "$LOG_DIR/conductor.pid"

echo "Starting Router on port 8502..."
"$ARKAVO_BIN" agent -c "$KIT" -n "council-router" -p 8502 > "$LOG_DIR/router.log" 2>&1 &
echo $! > "$LOG_DIR/router.pid"

echo "Starting Critic on port 8503..."
"$ARKAVO_BIN" agent -c "$KIT" -n "council-critic" -p 8503 > "$LOG_DIR/critic.log" 2>&1 &
echo $! > "$LOG_DIR/critic.pid"

echo "Starting Synthesis on port 8504..."
"$ARKAVO_BIN" agent -c "$KIT" -n "council-synthesis" -p 8504 > "$LOG_DIR/synthesis.log" 2>&1 &
echo $! > "$LOG_DIR/synthesis.pid"

# Launch specialist agents
echo "Starting Critical Analyst on port 8510..."
"$ARKAVO_BIN" agent -c "$KIT" -n "critical-analyst" -p 8510 > "$LOG_DIR/critical-analyst.log" 2>&1 &
echo $! > "$LOG_DIR/critical-analyst.pid"

echo "Starting Researcher on port 8511..."
"$ARKAVO_BIN" agent -c "$KIT" -n "researcher" -p 8511 > "$LOG_DIR/researcher.log" 2>&1 &
echo $! > "$LOG_DIR/researcher.pid"

echo "Starting Synthesizer on port 8512..."
"$ARKAVO_BIN" agent -c "$KIT" -n "synthesizer" -p 8512 > "$LOG_DIR/synthesizer.log" 2>&1 &
echo $! > "$LOG_DIR/synthesizer.pid"

echo "Starting Devil's Advocate on port 8513..."
"$ARKAVO_BIN" agent -c "$KIT" -n "devils-advocate" -p 8513 > "$LOG_DIR/devils-advocate.log" 2>&1 &
echo $! > "$LOG_DIR/devils-advocate.pid"

echo "Starting Facilitator on port 8514..."
"$ARKAVO_BIN" agent -c "$KIT" -n "facilitator" -p 8514 > "$LOG_DIR/facilitator.log" 2>&1 &
echo $! > "$LOG_DIR/facilitator.pid"

echo ""
echo "AI Council mesh launched!"
echo "  Conductor:        http://localhost:8501"
echo "  Router:           http://localhost:8502"
echo "  Critic:           http://localhost:8503"
echo "  Synthesis:        http://localhost:8504"
echo "  Critical Analyst: http://localhost:8510"
echo "  Researcher:       http://localhost:8511"
echo "  Synthesizer:      http://localhost:8512"
echo "  Devil's Advocate: http://localhost:8513"
echo "  Facilitator:      http://localhost:8514"
echo ""
echo "Logs available in: $LOG_DIR"
echo "To stop: ./stop_mesh.sh"
