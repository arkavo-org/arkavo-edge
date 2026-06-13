#!/bin/bash
# Project Zomboid — single survivor agent against the Game-RL mod (file IPC)
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
if [ ! -f "$BINARY" ]; then
    BINARY="${PROJECT_ROOT}/target/release/arkavo"
fi
LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p "$LOG_DIR"

# game-rl-server auto-detects Project Zomboid via the Lua file IPC
# (~/Zomboid/Lua/gamerl_response.json). Resolution: env, PATH, sibling repo.
export GAME_RL_SERVER="${GAME_RL_SERVER:-$(command -v game-rl-server || echo "$HOME/Projects/intelligence/game-rl/target/release/game-rl-server")}"

case "${1:-start}" in
    stop)
        pkill -f "arkavo agent.*zomboid" 2>/dev/null || true
        pkill -f "agents/survivor" 2>/dev/null || true
        echo "[OK] stopped"
        exit 0
        ;;
esac

if [ ! -S /tmp/.gamerl 2>/dev/null ] && [ ! -f "$HOME/Zomboid/Lua/gamerl_response.json" ]; then
    echo "[WARN] Project Zomboid GameRL IPC not found at ~/Zomboid/Lua/"
    echo "       Start Project Zomboid with the GameRL mod and load a save first."
fi

echo "[INFO] server:   $GAME_RL_SERVER"
echo "[INFO] starting survivor on :8451 (logs: $LOG_DIR/survivor.log)"

cd "$SCRIPT_DIR/agents/survivor"
exec "$BINARY" agent --config AGENTS.md 2>&1 | tee "$LOG_DIR/survivor.log"
