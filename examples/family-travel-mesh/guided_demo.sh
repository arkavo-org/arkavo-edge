#!/bin/bash
# Family Travel Mesh - Guided Demo with Real LLM Calls
# Demonstrates: arkavo task → Conductor → Router → Specialist → Critic

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${PROJECT_ROOT}/target/debug/arkavo"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

USER_PROMPT="Top 3 Vegas recommendations for a family with toddlers?"
TEMP_FILE=$(mktemp)

# ═══════════════════════════════════════════════════════════════════════
# STEP 1: Check mesh is running
# ═══════════════════════════════════════════════════════════════════════
check_mesh() {
    echo ""
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}   HRM GUIDED DEMO - Agent Mesh Flow                                  ${NC}"
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""

    echo -e "${YELLOW}[MESH CHECK]${NC} Verifying agents are running..."

    # Check if conductor is listening
    if ! curl -s http://localhost:8401/.well-known/agent.json > /dev/null 2>&1; then
        echo -e "${RED}Error: Conductor not running on port 8401${NC}"
        echo "Start the mesh first: ./launch_mesh.sh"
        exit 1
    fi

    echo -e "  ${GREEN}✓${NC} Conductor (8401)"

    # Check other agents
    for port in 8402 8410 8411 8412; do
        if curl -s http://localhost:$port/.well-known/agent.json > /dev/null 2>&1; then
            echo -e "  ${GREEN}✓${NC} Agent on port $port"
        else
            echo -e "  ${YELLOW}!${NC} Agent on port $port not responding"
        fi
    done
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 2: Show the request
# ═══════════════════════════════════════════════════════════════════════
show_request() {
    echo -e "${YELLOW}[USER REQUEST]${NC}"
    echo -e "  \"$USER_PROMPT\""
    echo ""
    echo -e "${DIM}Flow: arkavo task → Conductor → Router → Specialist → Critic${NC}"
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 3: Send task to mesh via conductor
# ═══════════════════════════════════════════════════════════════════════
send_to_mesh() {
    echo -e "${DIM}───────────────────────────────────────────────────────────────────────${NC}"
    echo ""
    echo -e "${CYAN}[TASK]${NC} Sending to mesh via conductor..."
    echo ""

    # Use arkavo task with mesh-only flag to route through conductor
    cd "$SCRIPT_DIR"

    RESPONSE=$(timeout 120 "$BINARY" task --mesh-only "$USER_PROMPT" 2>&1 || echo "Error: Task failed")

    echo -e "${DIM}Response:${NC}"
    echo "$RESPONSE"
    echo ""

    # Store for analysis
    echo "$RESPONSE" > "$TEMP_FILE"
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 4: Analyze response for policy violations
# ═══════════════════════════════════════════════════════════════════════
analyze_response() {
    echo -e "${DIM}───────────────────────────────────────────────────────────────────────${NC}"
    echo ""

    RESPONSE=$(cat "$TEMP_FILE")

    # Check for policy violations
    if echo "$RESPONSE" | grep -iq "casino\|gambling\|slot machine\|poker\|blackjack\|roulette"; then
        echo -e "  ${BOLD}${RED}╔═══════════════════════════════════════════════════════════════╗${NC}"
        echo -e "  ${BOLD}${RED}║  🛑 POLICY VIOLATION DETECTED                                 ║${NC}"
        echo -e "  ${BOLD}${RED}║  Casino/gambling content in response                          ║${NC}"
        echo -e "  ${BOLD}${RED}║  Critic should have vetoed this response                      ║${NC}"
        echo -e "  ${BOLD}${RED}╚═══════════════════════════════════════════════════════════════╝${NC}"
    else
        echo -e "  ${BOLD}${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
        echo -e "  ${BOLD}${GREEN}║  ✅ RESPONSE APPROVED                                         ║${NC}"
        echo -e "  ${BOLD}${GREEN}║  No policy violations detected                                ║${NC}"
        echo -e "  ${BOLD}${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    fi
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 5: Show result
# ═══════════════════════════════════════════════════════════════════════
show_result() {
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${GREEN}   DEMO COMPLETED                                                      ${NC}"
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo -e "${BOLD}Flow executed:${NC}"
    echo -e "  ${CYAN}1.${NC} arkavo task sent prompt to mesh"
    echo -e "  ${CYAN}2.${NC} Conductor received and decomposed task"
    echo -e "  ${CYAN}3.${NC} Router selected appropriate specialist"
    echo -e "  ${CYAN}4.${NC} Specialist generated response"
    echo -e "  ${CYAN}5.${NC} Critic evaluated for policy compliance"
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════════════
main() {
    # Check binary exists
    if [ ! -f "$BINARY" ]; then
        echo -e "${RED}Error: Binary not found at $BINARY${NC}"
        echo "Run: cargo build"
        exit 1
    fi

    check_mesh

    echo -e "${DIM}Press Enter to send task to mesh...${NC}"
    read -r

    show_request
    send_to_mesh

    echo -e "${DIM}Press Enter to analyze response...${NC}"
    read -r

    analyze_response
    show_result

    # Cleanup
    rm -f "$TEMP_FILE"
}

main "$@"
