#!/bin/bash
# stop_minecraft.sh - Stop Minecraft swarm agents and server

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIDS_FILE="$SCRIPT_DIR/.pids"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}[SWARM]${NC} Stopping agents..."

# Stop agents by PID
if [ -f "$PIDS_FILE" ]; then
    while IFS=':' read -r pid name; do
        if kill -0 "$pid" 2>/dev/null; then
            echo -e "  Stopping $name (PID: $pid)"
            kill "$pid" 2>/dev/null || true
        fi
    done < "$PIDS_FILE"
    rm -f "$PIDS_FILE"
fi

# Fallback: kill any remaining arkavo agent processes
pkill -f "arkavo agent" 2>/dev/null || true

echo -e "${YELLOW}[MINECRAFT]${NC} Stopping server..."

cd "$SCRIPT_DIR"
docker compose down

echo -e "${GREEN}[SWARM]${NC} All stopped."
