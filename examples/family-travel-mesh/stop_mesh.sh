#!/bin/bash
# Family Travel Mesh - Stop Script

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIDS_FILE="${SCRIPT_DIR}/.pids"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}[MESH]${NC} Stopping all agents..."

if [ -f "$PIDS_FILE" ]; then
    while IFS=':' read -r pid name; do
        if kill -0 "$pid" 2>/dev/null; then
            echo -e "${BLUE}[MESH]${NC} Stopping $name (PID: $pid)..."
            kill "$pid" 2>/dev/null || true
        fi
    done < "$PIDS_FILE"
    rm -f "$PIDS_FILE"
    echo -e "${GREEN}[SUCCESS]${NC} All agents stopped"
else
    echo -e "${BLUE}[MESH]${NC} No running agents found"
fi
