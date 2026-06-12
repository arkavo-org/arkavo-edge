#!/bin/bash
# Zomboid Survival Kit — run the SwarmKit live against Project Zomboid.
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
[ -f "$BINARY" ] || BINARY="${PROJECT_ROOT}/target/release/arkavo"

# game-rl-server auto-detects Project Zomboid via ~/Zomboid/Lua/ file IPC.
# Resolution: explicit env -> PATH -> sibling game-rl repo build.
export GAME_RL_SERVER="${GAME_RL_SERVER:-$(command -v game-rl-server || echo "$HOME/Projects/intelligence/game-rl/target/release/game-rl-server")}"

KIT="$SCRIPT_DIR/zomboid-survival-kit.swarmkit.yaml"

if [ ! -x "$BINARY" ]; then
    echo "[ERROR] arkavo binary not found at $BINARY — run: cargo build -p arkavo"
    exit 1
fi
if [ ! -x "$GAME_RL_SERVER" ]; then
    echo "[WARN] game-rl-server not executable at $GAME_RL_SERVER (set GAME_RL_SERVER or build it in the game-rl repo)"
fi
if [ ! -f "$HOME/Zomboid/Lua/gamerl_response.json" ]; then
    echo "[WARN] Project Zomboid GameRL IPC not found — start Project Zomboid (GameRL mod, save loaded) first."
fi

echo "[INFO] server: $GAME_RL_SERVER"
echo "[INFO] kit:    $KIT"
exec "$BINARY" swarmkit play "$KIT"
