#!/bin/bash
# Launch HYPERforum AI Council mesh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR"

echo "Launching HYPERforum AI Council..."

# Launch infrastructure agents
echo "Starting Conductor on port 8501..."
arkavo agent start --config "$SCRIPT_DIR/agents/conductor/AGENTS.md" > "$LOG_DIR/conductor.log" 2>&1 &
echo $! > "$LOG_DIR/conductor.pid"

echo "Starting Router on port 8502..."
arkavo agent start --config "$SCRIPT_DIR/agents/router/AGENTS.md" > "$LOG_DIR/router.log" 2>&1 &
echo $! > "$LOG_DIR/router.pid"

echo "Starting Critic on port 8503..."
arkavo agent start --config "$SCRIPT_DIR/agents/critic/AGENTS.md" > "$LOG_DIR/critic.log" 2>&1 &
echo $! > "$LOG_DIR/critic.pid"

echo "Starting Synthesis on port 8504..."
arkavo agent start --config "$SCRIPT_DIR/agents/synthesis/AGENTS.md" > "$LOG_DIR/synthesis.log" 2>&1 &
echo $! > "$LOG_DIR/synthesis.pid"

# Launch specialist agents
echo "Starting Critical Analyst on port 8510..."
arkavo agent start --config "$SCRIPT_DIR/agents/specialists/critical-analyst/AGENTS.md" > "$LOG_DIR/critical-analyst.log" 2>&1 &
echo $! > "$LOG_DIR/critical-analyst.pid"

echo "Starting Researcher on port 8511..."
arkavo agent start --config "$SCRIPT_DIR/agents/specialists/researcher/AGENTS.md" > "$LOG_DIR/researcher.log" 2>&1 &
echo $! > "$LOG_DIR/researcher.pid"

echo "Starting Synthesizer on port 8512..."
arkavo agent start --config "$SCRIPT_DIR/agents/specialists/synthesizer/AGENTS.md" > "$LOG_DIR/synthesizer.log" 2>&1 &
echo $! > "$LOG_DIR/synthesizer.pid"

echo "Starting Devil's Advocate on port 8513..."
arkavo agent start --config "$SCRIPT_DIR/agents/specialists/devils-advocate/AGENTS.md" > "$LOG_DIR/devils-advocate.log" 2>&1 &
echo $! > "$LOG_DIR/devils-advocate.pid"

echo "Starting Facilitator on port 8514..."
arkavo agent start --config "$SCRIPT_DIR/agents/specialists/facilitator/AGENTS.md" > "$LOG_DIR/facilitator.log" 2>&1 &
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
