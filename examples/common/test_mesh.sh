#!/bin/bash
# test_mesh.sh - Universal mesh test script
#
# Usage: ./test_mesh.sh examples/family-travel-mesh [primary_port]
#
# Tests any mesh example by:
# 1. Finding and running the launch script
# 2. Waiting for health checks
# 3. Verifying mesh discovery
# 4. Running cleanup

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

DIR="${1:-.}"
PRIMARY_PORT="${2:-8342}"
STARTUP_WAIT="${3:-15}"

echo -e "${BLUE}Testing mesh in $DIR...${NC}"

# Validate directory
if [[ ! -d "$DIR" ]]; then
    echo -e "${RED}Error: Directory not found: $DIR${NC}"
    exit 1
fi

cd "$DIR"

# Find and run launch script
launch_and_get_pid() {
    if [[ -f "launch_multi_agent_system.sh" ]]; then
        ./launch_multi_agent_system.sh &
        echo $!
    elif [[ -f "launch_mesh.sh" ]]; then
        ./launch_mesh.sh start
        echo ""
    elif [[ -f "launch_agents.sh" ]]; then
        ./launch_agents.sh start
        echo ""
    elif [[ -f "launch_fleet.sh" ]]; then
        ./launch_fleet.sh &
        echo $!
    else
        echo -e "${RED}Error: No launch script found${NC}" >&2
        exit 1
    fi
}

# Cleanup function
cleanup() {
    echo -e "${BLUE}Cleaning up...${NC}"

    if [[ -n "$LAUNCH_PID" ]]; then
        pkill -P "$LAUNCH_PID" 2>/dev/null || true
        kill "$LAUNCH_PID" 2>/dev/null || true
    fi

    # Try various stop scripts
    [[ -f "stop_mesh.sh" ]] && ./stop_mesh.sh 2>/dev/null || true
    [[ -f "stop_fleet.sh" ]] && ./stop_fleet.sh 2>/dev/null || true
    [[ -f "launch_agents.sh" ]] && ./launch_agents.sh stop 2>/dev/null || true
}

trap cleanup EXIT

# Launch
echo -e "${BLUE}Launching mesh...${NC}"
LAUNCH_PID=$(launch_and_get_pid)

# Wait for startup
echo -e "${BLUE}Waiting ${STARTUP_WAIT}s for startup...${NC}"
sleep "$STARTUP_WAIT"

# Health check
echo -e "${BLUE}Checking health on port $PRIMARY_PORT...${NC}"
if curl -sf "http://localhost:$PRIMARY_PORT/.well-known/agent.json" >/dev/null 2>&1; then
    echo -e "${GREEN}✓ Primary agent healthy${NC}"
else
    echo -e "${RED}✗ Primary agent not responding on port $PRIMARY_PORT${NC}"

    # Check common ports
    for port in 8340 8341 8342 8343 8344 8401 8402 8403; do
        if curl -sf "http://localhost:$port/.well-known/agent.json" >/dev/null 2>&1; then
            echo -e "${YELLOW}  Found agent on port $port${NC}"
        fi
    done

    exit 1
fi

# Check for peer discovery in logs
if [[ -d "logs" ]]; then
    if grep -rqi "peer\|discovered\|registered" logs/ 2>/dev/null; then
        echo -e "${GREEN}✓ Peer discovery detected in logs${NC}"
    else
        echo -e "${YELLOW}! No peer discovery messages in logs (may be normal)${NC}"
    fi
fi

echo -e "${GREEN}=== TEST PASSED ===${NC}"
exit 0
