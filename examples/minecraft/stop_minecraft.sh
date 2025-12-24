#!/bin/bash
# stop_minecraft.sh - Stop Minecraft server and agent

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}[MINECRAFT]${NC} Stopping Minecraft demo..."

# Stop any arkavo agent processes in this directory
pkill -f "arkavo agent" 2>/dev/null || true

# Stop Docker containers
cd "$SCRIPT_DIR"
if docker compose ps 2>/dev/null | grep -q "minecraft"; then
    echo -e "${YELLOW}[MINECRAFT]${NC} Stopping Minecraft server..."
    docker compose down
fi

echo -e "${GREEN}[MINECRAFT]${NC} Stopped."
