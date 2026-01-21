#!/bin/bash
# Hello World Agent - Your first Arkavo agent
#
# This script starts a single agent that responds to a simple greeting.
# Run time: ~5 minutes (including first-time model download)
#
# Usage:
#   ./run.sh              # Run with default model
#   ./run.sh --model glm  # Run with GLM-4.7-Flash

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse arguments
MODEL_FLAG=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --model|-m)
            MODEL_FLAG="--model $2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

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

if [ -n "$MODEL_FLAG" ]; then
    echo -e "${CYAN}Model:${NC} ${MODEL_FLAG#--model }"
fi
echo -e "${GREEN}Starting hello-agent...${NC}"
echo ""

# Run a simple chat query (no repo context for simple greeting)
cd "$SCRIPT_DIR"
# shellcheck disable=SC2086
"$BINARY" chat $MODEL_FLAG --repo-context off --prompt "Hello! Please introduce yourself briefly. What are you and what can you help with?"
