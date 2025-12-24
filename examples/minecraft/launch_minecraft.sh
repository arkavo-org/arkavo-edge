#!/bin/bash
# launch_minecraft.sh - Start Minecraft server for Arkavo Edge

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${GREEN}[MINECRAFT]${NC} Starting Minecraft server..."

# Check Docker
if ! docker info &> /dev/null; then
    echo -e "${RED}ERROR:${NC} Docker daemon is not running"
    exit 1
fi

cd "$SCRIPT_DIR"

# Start Minecraft server
docker compose up -d

echo -e "${YELLOW}[MINECRAFT]${NC} Waiting for server to be healthy..."

# Wait for minecraft to be healthy
for i in {1..90}; do
    if docker compose ps minecraft 2>/dev/null | grep -q "healthy"; then
        echo -e "\n${GREEN}[MINECRAFT]${NC} Server is healthy!"
        break
    fi
    if [ $i -eq 90 ]; then
        echo -e "\n${RED}ERROR:${NC} Server failed to become healthy"
        docker compose logs minecraft | tail -20
        exit 1
    fi
    echo -n "."
    sleep 2
done

echo ""
echo -e "${GREEN}[MINECRAFT]${NC} Server ready on localhost:25565"
echo ""
echo -e "Connect a Minecraft client to localhost:25565"
echo ""
echo -e "Stop server: ${YELLOW}./stop_minecraft.sh${NC}"

# Start agent
echo -e "${GREEN}[MINECRAFT]${NC} Starting agent..."
"$BINARY" agent run 2>&1 | tee logs/minecraft-agent.log
