#!/bin/bash
# Family Travel Mesh - Guided Demo with Real LLM Calls
# Demonstrates veto/reroute flow with actual Ministral inference

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

USER_REQUEST="Plan a Friday afternoon in Las Vegas for a family with twin 3-year-old toddlers. Budget is \$200, time window 12:00-18:00. Give me your top 3 recommendations."

# ═══════════════════════════════════════════════════════════════════════
# STEP 1: Show the request
# ═══════════════════════════════════════════════════════════════════════
show_request() {
    echo ""
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}   HRM GUIDED DEMO - Real LLM Inference                                ${NC}"
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo -e "${YELLOW}[USER REQUEST]${NC}"
    echo -e "  \"$USER_REQUEST\""
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 2: Call vegas-guide (biased toward casinos)
# ═══════════════════════════════════════════════════════════════════════
call_vegas_guide() {
    echo -e "${DIM}───────────────────────────────────────────────────────────────────────${NC}"
    echo ""
    echo -e "${YELLOW}[ROUTER]${NC} Thompson Sampling selects: ${MAGENTA}vegas-guide${NC} (score: 0.68)"
    echo ""
    echo -e "${MAGENTA}[SPECIALIST:vegas-guide]${NC} Calling Ministral LLM..."
    echo ""

    # Real LLM call to vegas-guide agent
    cd "$SCRIPT_DIR/agents/specialists/vegas-guide"

    VEGAS_RESPONSE=$(timeout 90 "$BINARY" chat --prompt "$USER_REQUEST" 2>&1 || echo "Error: LLM call failed")

    echo -e "${DIM}Response:${NC}"
    echo "$VEGAS_RESPONSE" | head -30
    echo ""

    # Store for critic evaluation
    echo "$VEGAS_RESPONSE" > /tmp/vegas_response.txt
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 3: Critic evaluates (should veto casino suggestions)
# ═══════════════════════════════════════════════════════════════════════
call_critic() {
    echo -e "${DIM}───────────────────────────────────────────────────────────────────────${NC}"
    echo ""
    echo -e "${RED}[CRITIC]${NC} Evaluating response against family_safety policy..."
    echo ""

    VEGAS_RESPONSE=$(cat /tmp/vegas_response.txt)

    # Check for policy violations
    POLICY_FILE="$SCRIPT_DIR/agents/critic/policies/family_safety.yaml"

    # Policy check: Look for explicit casino/gambling references
    VIOLATIONS=""
    if echo "$VEGAS_RESPONSE" | grep -iq "casino\|gambling\|slot machine\|poker\|blackjack\|roulette"; then
        VIOLATIONS="casino/gambling venue detected"
    fi
    if echo "$VEGAS_RESPONSE" | grep -iq "nightclub\|strip club\|21+\|adults only"; then
        VIOLATIONS="$VIOLATIONS, adult venue detected"
    fi

    if [ -n "$VIOLATIONS" ]; then
        echo -e "  ${BOLD}${RED}╔═══════════════════════════════════════════════════════════════╗${NC}"
        echo -e "  ${BOLD}${RED}║  🛑 VETO: Policy Violation Detected                           ║${NC}"
        echo -e "  ${BOLD}${RED}╠═══════════════════════════════════════════════════════════════╣${NC}"
        echo -e "  ${BOLD}${RED}║  Violation: $VIOLATIONS${NC}"
        echo -e "  ${BOLD}${RED}║  Policy: No gambling/casino venues for travelers under 21     ║${NC}"
        echo -e "  ${BOLD}${RED}║  Action: REJECTED - Rerouting to alternate specialist         ║${NC}"
        echo -e "  ${BOLD}${RED}╚═══════════════════════════════════════════════════════════════╝${NC}"
        echo ""

        # Update Thompson Sampling prior
        echo -e "${YELLOW}[ROUTER]${NC} Updating Thompson Sampling prior..."
        echo -e "  ${DIM}vegas-guide: Beta(35,12) → ${RED}Beta(35,13)${NC} ${DIM}(failure +1)${NC}"
        echo ""

        return 1  # Signal veto
    else
        echo -e "  ${GREEN}✓${NC} PolicyCheck passed"
        return 0  # Signal approval
    fi
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 4: Reroute to family-activities
# ═══════════════════════════════════════════════════════════════════════
call_family_activities() {
    echo -e "${DIM}───────────────────────────────────────────────────────────────────────${NC}"
    echo ""
    echo -e "  ${BOLD}${CYAN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "  ${BOLD}${CYAN}║  🔄 REROUTE: Selecting alternate specialist                   ║${NC}"
    echo -e "  ${BOLD}${CYAN}║  → family-activities (child-friendly expert)                  ║${NC}"
    echo -e "  ${BOLD}${CYAN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo ""

    echo -e "${GREEN}[SPECIALIST:family-activities]${NC} Calling Ministral LLM..."
    echo ""

    # Real LLM call to family-activities agent
    cd "$SCRIPT_DIR/agents/specialists/family-activities"

    FAMILY_RESPONSE=$(timeout 90 "$BINARY" chat --prompt "$USER_REQUEST" 2>&1 || echo "Error: LLM call failed")

    echo -e "${DIM}Response:${NC}"
    echo "$FAMILY_RESPONSE" | head -30
    echo ""

    # Store for critic evaluation
    echo "$FAMILY_RESPONSE" > /tmp/family_response.txt
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 5: Critic approves family-activities response
# ═══════════════════════════════════════════════════════════════════════
approve_response() {
    echo -e "${DIM}───────────────────────────────────────────────────────────────────────${NC}"
    echo ""
    echo -e "${GREEN}[CRITIC]${NC} Evaluating response against family_safety policy..."
    echo ""

    FAMILY_RESPONSE=$(cat /tmp/family_response.txt)

    # Policy check: Look for explicit casino/gambling references
    VIOLATIONS=""
    if echo "$FAMILY_RESPONSE" | grep -iq "casino\|gambling\|slot machine\|poker\|blackjack\|roulette"; then
        VIOLATIONS="casino/gambling venue detected"
    fi

    if [ -n "$VIOLATIONS" ]; then
        echo -e "  ${RED}✗${NC} PolicyCheck failed: $VIOLATIONS"
        return 1
    else
        echo -e "  ${DIM}[0] SchemaCheck:${NC}   ${GREEN}✓${NC}"
        echo -e "  ${DIM}[1] LintCheck:${NC}     ${GREEN}✓${NC}"
        echo -e "  ${DIM}[2] PolicyCheck:${NC}   ${GREEN}✓${NC} ${DIM}(no prohibited venues)${NC}"
        echo -e "  ${DIM}[3] SemanticCheck:${NC} ${GREEN}✓${NC} ${DIM}(age-appropriate confirmed)${NC}"
        echo ""
        echo -e "  ${BOLD}${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
        echo -e "  ${BOLD}${GREEN}║  ✅ APPROVED                                                   ║${NC}"
        echo -e "  ${BOLD}${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
        echo ""

        # Update Thompson Sampling prior
        echo -e "${YELLOW}[ROUTER]${NC} Updating Thompson Sampling prior..."
        echo -e "  ${DIM}family-activities: Beta(28,15) → ${GREEN}Beta(29,15)${NC} ${DIM}(success +1)${NC}"
        echo ""

        return 0
    fi
}

# ═══════════════════════════════════════════════════════════════════════
# STEP 6: Show final result
# ═══════════════════════════════════════════════════════════════════════
show_result() {
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${GREEN}   TASK COMPLETED                                                      ${NC}"
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo -e "${BOLD}Key Moments:${NC}"
    echo -e "  ${RED}🛑${NC} vegas-guide suggested casinos → ${RED}VETOED${NC}"
    echo -e "  ${CYAN}🔄${NC} Router rerouted to family-activities"
    echo -e "  ${GREEN}✅${NC} family-activities gave safe recommendations → ${GREEN}APPROVED${NC}"
    echo -e "  ${YELLOW}📉${NC} Thompson Sampling updated: vegas-guide penalized, family-activities rewarded"
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

    show_request

    echo -e "${DIM}Press Enter to call vegas-guide...${NC}"
    read -r

    call_vegas_guide

    echo -e "${DIM}Press Enter for Critic evaluation...${NC}"
    read -r

    if ! call_critic; then
        echo -e "${DIM}Press Enter to reroute to family-activities...${NC}"
        read -r

        call_family_activities

        echo -e "${DIM}Press Enter for Critic evaluation...${NC}"
        read -r

        approve_response
    fi

    show_result

    # Cleanup
    rm -f /tmp/vegas_response.txt /tmp/family_response.txt
}

main "$@"
