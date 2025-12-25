#!/bin/bash
# stop_minecraft.sh - Stop Minecraft server and MCP bot

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}[MINECRAFT]${NC} Stopping Minecraft server and MCP bot..."

cd "$SCRIPT_DIR"
docker compose down

echo -e "${GREEN}[MINECRAFT]${NC} Stopped."
