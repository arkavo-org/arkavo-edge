#!/bin/bash
# check_prerequisites.sh - Shared prerequisite validation for examples
#
# Usage: source this file from your launch script
#   source "$(dirname "$0")/../common/check_prerequisites.sh"
#   check_arkavo_binary

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Find project root by looking for Cargo.toml
find_project_root() {
    local dir="$1"
    while [[ "$dir" != "/" ]]; do
        if [[ -f "$dir/Cargo.toml" ]] && grep -q "arkavo" "$dir/Cargo.toml" 2>/dev/null; then
            echo "$dir"
            return 0
        fi
        dir="$(dirname "$dir")"
    done
    return 1
}

# Check if arkavo binary exists
check_arkavo_binary() {
    local script_dir="${1:-$(pwd)}"
    local project_root

    project_root=$(find_project_root "$script_dir") || {
        echo -e "${RED}Error: Could not find project root${NC}"
        return 1
    }

    local binary="$project_root/target/debug/arkavo"

    if [[ ! -f "$binary" ]]; then
        echo -e "${RED}Error: Arkavo binary not found at $binary${NC}"
        echo -e "${YELLOW}Run: cargo build${NC}"
        return 1
    fi

    echo -e "${GREEN}✓${NC} Arkavo binary found"
    echo "$binary"
}

# Check if a port is available
check_port() {
    local port="$1"
    if lsof -Pi ":$port" -sTCP:LISTEN -t >/dev/null 2>&1; then
        echo -e "${RED}Error: Port $port is already in use${NC}"
        lsof -Pi ":$port" -sTCP:LISTEN
        return 1
    fi
    return 0
}

# Check multiple ports
check_ports() {
    local ports=("$@")
    local failed=0

    for port in "${ports[@]}"; do
        if ! check_port "$port"; then
            failed=1
        fi
    done

    if [[ $failed -eq 1 ]]; then
        echo -e "${YELLOW}Tip: Kill existing processes or choose different ports${NC}"
        return 1
    fi

    echo -e "${GREEN}✓${NC} All ports available"
    return 0
}

# Check if npm is installed (for MCP servers that require it)
check_npm() {
    if ! command -v npm &>/dev/null; then
        echo -e "${YELLOW}Warning: npm not found (some MCP servers may not work)${NC}"
        return 1
    fi
    echo -e "${GREEN}✓${NC} npm found: $(npm --version)"
    return 0
}

# Check if npx is installed
check_npx() {
    if ! command -v npx &>/dev/null; then
        echo -e "${YELLOW}Warning: npx not found${NC}"
        return 1
    fi
    echo -e "${GREEN}✓${NC} npx found"
    return 0
}

# Health check for an agent endpoint (uses /.well-known/agent.json per A2A spec)
check_health() {
    local url="$1"
    local name="${2:-Agent}"
    local timeout="${3:-5}"

    if curl -sf --connect-timeout "$timeout" "$url/.well-known/agent.json" >/dev/null 2>&1; then
        echo -e "${GREEN}✓${NC} $name is healthy"
        return 0
    else
        echo -e "${RED}✗${NC} $name is not responding"
        return 1
    fi
}

# Wait for agent to become healthy (uses /.well-known/agent.json per A2A spec)
wait_for_health() {
    local url="$1"
    local name="${2:-Agent}"
    local max_attempts="${3:-30}"
    local interval="${4:-1}"

    echo -n "Waiting for $name..."
    for ((i=1; i<=max_attempts; i++)); do
        if curl -sf --connect-timeout 2 "$url/.well-known/agent.json" >/dev/null 2>&1; then
            echo -e " ${GREEN}OK${NC}"
            return 0
        fi
        echo -n "."
        sleep "$interval"
    done
    echo -e " ${RED}TIMEOUT${NC}"
    return 1
}
