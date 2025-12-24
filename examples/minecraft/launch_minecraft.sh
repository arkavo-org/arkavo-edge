#!/bin/bash
# launch_minecraft.sh - Start Minecraft server and agent

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}[MINECRAFT]${NC} Starting Minecraft demo..."

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}ERROR:${NC} Build arkavo first: cargo build"
    exit 1
fi

if ! command -v docker &> /dev/null; then
    echo -e "${RED}ERROR:${NC} Docker is required. Install from https://docker.com"
    exit 1
fi

if ! docker info &> /dev/null; then
    echo -e "${RED}ERROR:${NC} Docker daemon is not running"
    exit 1
fi

if ! command -v node &> /dev/null; then
    echo -e "${RED}ERROR:${NC} Node.js is required. Install version 20+"
    exit 1
fi

NODE_VERSION=$(node --version | cut -d'v' -f2 | cut -d'.' -f1)
if [ "$NODE_VERSION" -lt 20 ]; then
    echo -e "${RED}ERROR:${NC} Node.js 20+ required, found v${NODE_VERSION}"
    exit 1
fi

# Create logs directory
mkdir -p "$SCRIPT_DIR/logs"

# Start Minecraft server if not running
cd "$SCRIPT_DIR"
if ! docker compose ps minecraft 2>/dev/null | grep -q "running"; then
    echo -e "${YELLOW}[MINECRAFT]${NC} Starting Minecraft server..."
    docker compose up -d minecraft

    echo -e "${YELLOW}[MINECRAFT]${NC} Waiting for server to be ready..."
    for i in {1..60}; do
        if docker compose logs minecraft 2>&1 | grep -q "Done"; then
            echo -e "${GREEN}[MINECRAFT]${NC} Server is ready!"
            break
        fi
        if [ $i -eq 60 ]; then
            echo -e "${RED}ERROR:${NC} Server failed to start in 60 seconds"
            docker compose logs minecraft
            exit 1
        fi
        sleep 1
        echo -n "."
    done
    echo
else
    echo -e "${GREEN}[MINECRAFT]${NC} Minecraft server already running"
fi

# Start agent
echo -e "${GREEN}[MINECRAFT]${NC} Starting agent..."
cd "$SCRIPT_DIR"
"$BINARY" agent run 2>&1 | tee logs/minecraft-agent.log
