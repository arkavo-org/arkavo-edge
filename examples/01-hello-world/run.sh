#!/bin/bash
# Hello World Agent - Your first Arkavo agent
#
# This script starts a single agent that responds to a simple greeting.
# Run time: ~5 minutes (including first-time model download)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

echo "Hello World Agent"
echo "================="
echo ""

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}Error: Arkavo binary not found${NC}"
    echo ""
    echo "Please build first:"
    echo "  cd $(dirname "$SCRIPT_DIR")"
    echo "  cargo build"
    exit 1
fi

echo -e "${GREEN}Starting hello-agent...${NC}"
echo ""

# Run a simple chat query (no repo context for simple greeting)
cd "$SCRIPT_DIR"
"$BINARY" chat --repo-context off --prompt "Hello! Please introduce yourself briefly. What are you and what can you help with?"
