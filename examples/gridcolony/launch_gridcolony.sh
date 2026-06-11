#!/bin/bash
# GridColony — single-agent Game-RL playtest against the reference environment
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
if [ ! -f "$BINARY" ]; then
    BINARY="${PROJECT_ROOT}/target/release/arkavo"
fi
LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR"

# The Game-RL reference environment binary (spec draft-02 Level 3).
# Resolution order: explicit env, PATH, sibling game-rl repo.
export GAME_RL_REFERENCE="${GAME_RL_REFERENCE:-$(command -v game-rl-reference || echo "$HOME/Projects/intelligence/game-rl/target/release/game-rl-reference")}"
# Scenario: default | fertile-corner | threat-south | scattered-resources (append +dr for randomization)
export GRIDCOLONY_SCENARIO="${GRIDCOLONY_SCENARIO:-fertile-corner}"

if [ ! -x "$GAME_RL_REFERENCE" ]; then
    echo "[ERROR] game-rl-reference not found (GAME_RL_REFERENCE=$GAME_RL_REFERENCE)"
    echo "        Build it: cargo build --release -p game-rl-reference  (in the game-rl repo)"
    exit 1
fi

case "${1:-start}" in
    stop)
        pkill -f "arkavo agent.*gridcolony" 2>/dev/null || true
        echo "[OK] stopped"
        exit 0
        ;;
esac

echo "[INFO] server:   $GAME_RL_REFERENCE"
echo "[INFO] scenario: $GRIDCOLONY_SCENARIO"
echo "[INFO] starting commander on :8431 (logs: $LOG_DIR/commander.log)"

cd "$SCRIPT_DIR/agents/commander"
exec "$BINARY" agent --config AGENTS.md 2>&1 | tee "$LOG_DIR/commander.log"
