#!/bin/bash
# Family Travel Mesh - HRM Demo with Narrative Arc
# Demonstrates: Problem → Solution → Proof → Differentiator

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

# Timing
PAUSE_SHORT=1
PAUSE_MED=2
PAUSE_LONG=3

pause() { sleep "${1:-$PAUSE_MED}"; }
type_text() {
    local text="$1"
    local delay="${2:-0.02}"
    for (( i=0; i<${#text}; i++ )); do
        printf "%s" "${text:$i:1}"
        sleep "$delay"
    done
    echo ""
}

clear_section() {
    echo ""
    echo -e "${DIM}─────────────────────────────────────────────────────────────────────${NC}"
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════
# ACT 1: THE PROBLEM
# ═══════════════════════════════════════════════════════════════════════
act1_problem() {
    clear
    echo ""
    echo -e "${BOLD}${RED}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${RED}   THE PROBLEM                                                         ${NC}"
    echo -e "${BOLD}${RED}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
    pause $PAUSE_SHORT

    echo -e "${YELLOW}AI agents are powerful... but dangerous without guardrails.${NC}"
    pause $PAUSE_MED
    echo ""

    echo -e "  ${DIM}•${NC} They drift off-task and blow budgets"
    pause $PAUSE_SHORT
    echo -e "  ${DIM}•${NC} They suggest inappropriate things"
    pause $PAUSE_SHORT
    echo -e "  ${DIM}•${NC} They have no concept of ${BOLD}your${NC} policies"
    pause $PAUSE_MED
    echo ""

    echo -e "${CYAN}Imagine asking an AI to plan a family trip to Vegas...${NC}"
    pause $PAUSE_MED
    echo ""

    echo -e "  ${RED}\"Here's my recommendation for your 3-year-old twins:${NC}"
    echo -e "  ${RED} The Bellagio Casino floor has amazing fountains!\"${NC}"
    pause $PAUSE_LONG
    echo ""

    echo -e "${BOLD}${YELLOW}What if your AI travel planner recommended a casino for your toddlers?${NC}"
    pause $PAUSE_LONG
}

# ═══════════════════════════════════════════════════════════════════════
# ACT 2: THE SOLUTION
# ═══════════════════════════════════════════════════════════════════════
act2_solution() {
    clear_section
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${GREEN}   THE SOLUTION: HRM Orchestration                                     ${NC}"
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
    pause $PAUSE_SHORT

    echo -e "${CYAN}Hierarchical Reasoning Model (HRM) provides:${NC}"
    echo ""
    pause $PAUSE_SHORT

    echo -e "  ${GREEN}✓${NC} ${BOLD}Bounded Execution${NC}"
    echo -e "    ${DIM}Agents have strict budgets (time, cost, steps)${NC}"
    pause $PAUSE_SHORT

    echo -e "  ${GREEN}✓${NC} ${BOLD}Specialist Routing${NC}"
    echo -e "    ${DIM}Thompson Sampling learns which agents to trust${NC}"
    pause $PAUSE_SHORT

    echo -e "  ${GREEN}✓${NC} ${BOLD}Safety Verification${NC}"
    echo -e "    ${DIM}A Critic enforces YOUR policies before approval${NC}"
    pause $PAUSE_MED
    echo ""

    echo -e "${MAGENTA}The Architecture:${NC}"
    echo ""
    echo -e "  ${BLUE}┌─────────────────────────────────────────────────────────────┐${NC}"
    echo -e "  ${BLUE}│${NC}  ${BOLD}User Request${NC}                                                ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}      ↓                                                       ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}  ${CYAN}CONDUCTOR${NC} (decomposes task)                                 ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}      ↓                                                       ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}  ${YELLOW}ROUTER${NC} (Thompson Sampling selects specialist)              ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}      ↓                                                       ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}  ${GREEN}SPECIALIST${NC} → ${RED}CRITIC${NC} (policy gate) → ${GREEN}✓ or 🛑${NC}              ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}      ↓                                                       ${BLUE}│${NC}"
    echo -e "  ${BLUE}│${NC}  ${BOLD}Safe Result${NC}                                                 ${BLUE}│${NC}"
    echo -e "  ${BLUE}└─────────────────────────────────────────────────────────────┘${NC}"
    pause $PAUSE_LONG
}

# ═══════════════════════════════════════════════════════════════════════
# ACT 3: THE PROOF (Live Demo)
# ═══════════════════════════════════════════════════════════════════════
act3_proof() {
    clear_section
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}   THE PROOF: Live Demo                                                ${NC}"
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""

    echo -e "${YELLOW}User Request:${NC}"
    echo -e "  \"Plan a Friday afternoon in Las Vegas for a family"
    echo -e "   with twin 3-year-old toddlers. Budget: \$200.\""
    echo ""
    pause $PAUSE_MED

    # Step 1: Conductor
    echo -e "${CYAN}[CONDUCTOR]${NC} Task received. Decomposing..."
    pause $PAUSE_SHORT
    echo -e "${DIM}  ├── Subtask 1: Find age-appropriate activities${NC}"
    echo -e "${DIM}  ├── Subtask 2: Verify safety and accessibility${NC}"
    echo -e "${DIM}  └── Subtask 3: Optimize for budget${NC}"
    echo ""
    pause $PAUSE_MED

    # Step 2: Router with Thompson Sampling
    echo -e "${YELLOW}[ROUTER]${NC} Thompson Sampling agent selection..."
    pause $PAUSE_SHORT
    echo ""
    echo -e "  ${DIM}Agent Priors (Beta distributions):${NC}"
    echo -e "  ┌────────────────────────┬────────────┬───────────┐"
    echo -e "  │ Agent                  │ Prior      │ Score     │"
    echo -e "  ├────────────────────────┼────────────┼───────────┤"
    echo -e "  │ ${MAGENTA}vegas-guide${NC}            │ Beta(35,12)│ ${BOLD}0.68${NC}      │"
    echo -e "  │ family-activities      │ Beta(28,15)│ 0.52      │"
    echo -e "  │ budget-optimizer       │ Beta(42,18)│ 0.61      │"
    echo -e "  └────────────────────────┴────────────┴───────────┘"
    echo ""
    echo -e "  ${YELLOW}→ Selected: vegas-guide (highest score)${NC}"
    echo ""
    pause $PAUSE_MED

    # Step 3: Vegas Guide suggests casino
    echo -e "${MAGENTA}[SPECIALIST:vegas-guide]${NC} Generating recommendations..."
    pause $PAUSE_MED
    echo ""
    echo -e "  ${DIM}Response:${NC}"
    echo -e "  \"For your Friday afternoon, I recommend:"
    echo -e "   ${RED}1. Bellagio Casino & Conservatory${NC} - World-famous fountains!"
    echo -e "   2. The LINQ Promenade - Great shopping"
    echo -e "   3. Fremont Street Experience - Live entertainment\""
    echo ""
    pause $PAUSE_MED

    # Step 4: THE VETO 🛑
    echo -e "${RED}[CRITIC]${NC} Verification pipeline..."
    pause $PAUSE_SHORT
    echo -e "  ${DIM}[0] SchemaCheck:${NC} ${GREEN}✓${NC} (0.8ms)"
    echo -e "  ${DIM}[1] LintCheck:${NC}   ${GREEN}✓${NC} (1.2ms)"
    echo -e "  ${DIM}[2] PolicyCheck:${NC} ${BOLD}...${NC}"
    pause $PAUSE_SHORT
    echo ""
    echo -e "  ${BOLD}${RED}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "  ${BOLD}${RED}║  🛑 VETO: Policy Violation Detected                           ║${NC}"
    echo -e "  ${BOLD}${RED}╠═══════════════════════════════════════════════════════════════╣${NC}"
    echo -e "  ${BOLD}${RED}║  Rule: No gambling venues for travelers under 21              ║${NC}"
    echo -e "  ${BOLD}${RED}║  Violation: \"Bellagio Casino\" in recommendations              ║${NC}"
    echo -e "  ${BOLD}${RED}║  Action: REJECTED - Rerouting to alternate specialist         ║${NC}"
    echo -e "  ${BOLD}${RED}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    pause $PAUSE_LONG

    # Step 5: Prior Update 📉
    echo -e "${YELLOW}[ROUTER]${NC} Updating Thompson Sampling priors..."
    echo ""
    echo -e "  ${DIM}vegas-guide:${NC} Beta(35,12) → ${RED}Beta(35,${BOLD}13${NC}${RED})${NC} ${DIM}(failure +1)${NC}"
    echo ""
    echo -e "  ${DIM}Updated Priors:${NC}"
    echo -e "  ┌────────────────────────┬────────────┬───────────┐"
    echo -e "  │ Agent                  │ Prior      │ Score     │"
    echo -e "  ├────────────────────────┼────────────┼───────────┤"
    echo -e "  │ vegas-guide            │ Beta(35,13)│ ${RED}0.65 ↓${NC}    │"
    echo -e "  │ ${GREEN}family-activities${NC}      │ Beta(28,15)│ ${BOLD}0.52${NC}      │"
    echo -e "  │ budget-optimizer       │ Beta(42,18)│ 0.61      │"
    echo -e "  └────────────────────────┴────────────┴───────────┘"
    echo ""
    pause $PAUSE_MED

    # Step 6: THE REROUTE 🔄
    echo -e "  ${BOLD}${CYAN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "  ${BOLD}${CYAN}║  🔄 REROUTE: Selecting alternate specialist                   ║${NC}"
    echo -e "  ${BOLD}${CYAN}║  → family-activities (child-friendly expert)                  ║${NC}"
    echo -e "  ${BOLD}${CYAN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    pause $PAUSE_MED

    # Step 7: Family Activities succeeds
    echo -e "${GREEN}[SPECIALIST:family-activities]${NC} Generating recommendations..."
    pause $PAUSE_MED
    echo ""
    echo -e "  ${DIM}Response:${NC}"
    echo -e "  \"Perfect afternoon for twin 3-year-olds:"
    echo -e "   ${GREEN}1. Discovery Children's Museum${NC} (\$15/person) - Interactive exhibits"
    echo -e "   ${GREEN}2. Adventuredome at Circus Circus${NC} (\$20/child) - Indoor theme park"
    echo -e "   ${GREEN}3. Shark Reef at Mandalay Bay${NC} (\$25/person) - Toddler-friendly\""
    echo ""
    echo -e "  ${DIM}Total estimated cost: \$153 (under \$200 budget)${NC}"
    echo ""
    pause $PAUSE_MED

    # Step 8: Critic approves
    echo -e "${GREEN}[CRITIC]${NC} Verification pipeline..."
    echo -e "  ${DIM}[0] SchemaCheck:${NC}   ${GREEN}✓${NC}"
    echo -e "  ${DIM}[1] LintCheck:${NC}     ${GREEN}✓${NC}"
    echo -e "  ${DIM}[2] PolicyCheck:${NC}   ${GREEN}✓${NC} ${DIM}(no prohibited venues)${NC}"
    echo -e "  ${DIM}[3] SemanticCheck:${NC} ${GREEN}✓${NC} ${DIM}(age-appropriate confirmed)${NC}"
    echo ""
    echo -e "  ${BOLD}${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "  ${BOLD}${GREEN}║  ✅ APPROVED                                                   ║${NC}"
    echo -e "  ${BOLD}${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    pause $PAUSE_MED

    # Step 9: Prior Update for success
    echo -e "${YELLOW}[ROUTER]${NC} Updating Thompson Sampling priors..."
    echo ""
    echo -e "  ${DIM}family-activities:${NC} Beta(28,15) → ${GREEN}Beta(${BOLD}29${NC}${GREEN},15)${NC} ${DIM}(success +1)${NC}"
    echo ""
    echo -e "  ${DIM}Final Priors:${NC}"
    echo -e "  ┌────────────────────────┬────────────┬───────────┐"
    echo -e "  │ Agent                  │ Prior      │ Score     │"
    echo -e "  ├────────────────────────┼────────────┼───────────┤"
    echo -e "  │ vegas-guide            │ Beta(35,13)│ 0.65      │"
    echo -e "  │ ${GREEN}family-activities${NC}      │ Beta(29,15)│ ${GREEN}0.66 ↑${NC}    │"
    echo -e "  │ budget-optimizer       │ Beta(42,18)│ 0.61      │"
    echo -e "  └────────────────────────┴────────────┴───────────┘"
    echo ""
    pause $PAUSE_MED

    # Final Result
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${GREEN}   FINAL RESULT                                                        ${NC}"
    echo -e "${BOLD}${GREEN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo -e "  ${BOLD}Your Vegas Friday Afternoon Plan:${NC}"
    echo ""
    echo -e "  12:00 - ${GREEN}Discovery Children's Museum${NC}"
    echo -e "          Interactive play areas, toddler zone"
    echo -e "          ${DIM}Cost: \$45 (2 adults + 2 children)${NC}"
    echo ""
    echo -e "  14:30 - ${GREEN}Adventuredome at Circus Circus${NC}"
    echo -e "          Indoor rides, carnival games"
    echo -e "          ${DIM}Cost: \$68 (family pass)${NC}"
    echo ""
    echo -e "  16:30 - ${GREEN}Shark Reef at Mandalay Bay${NC}"
    echo -e "          Touch pools, stroller-friendly"
    echo -e "          ${DIM}Cost: \$40 (2 adults, kids free under 4)${NC}"
    echo ""
    echo -e "  ${BOLD}Total: \$153${NC} ${DIM}(\$47 under budget)${NC}"
    echo ""
    pause $PAUSE_LONG
}

# ═══════════════════════════════════════════════════════════════════════
# ACT 4: THE DIFFERENTIATOR
# ═══════════════════════════════════════════════════════════════════════
act4_differentiator() {
    clear_section
    echo -e "${BOLD}${MAGENTA}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${MAGENTA}   THE DIFFERENTIATOR                                                  ${NC}"
    echo -e "${BOLD}${MAGENTA}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
    pause $PAUSE_SHORT

    echo -e "${CYAN}What makes HRM different:${NC}"
    echo ""

    echo -e "  ${MAGENTA}1. Thompson Sampling Learns Which Agents to Trust${NC}"
    echo -e "     ${DIM}Bad suggestions → lower prior → less likely to be selected${NC}"
    echo -e "     ${DIM}Good suggestions → higher prior → more likely to be selected${NC}"
    echo ""
    pause $PAUSE_SHORT

    echo -e "  ${MAGENTA}2. Bad Agents Get Penalized Automatically${NC}"
    echo -e "     ${DIM}No manual configuration needed${NC}"
    echo -e "     ${DIM}System learns from every interaction${NC}"
    echo ""
    pause $PAUSE_SHORT

    echo -e "  ${MAGENTA}3. System Self-Heals Without Human Intervention${NC}"
    echo -e "     ${DIM}Veto → Reroute → Retry with better agent${NC}"
    echo -e "     ${DIM}All automatic, all auditable${NC}"
    echo ""
    pause $PAUSE_MED

    echo -e "${BOLD}${GREEN}The Result:${NC}"
    echo -e "  ${GREEN}✓${NC} Policy enforcement is ${BOLD}tangible${NC} (you saw the veto)"
    echo -e "  ${GREEN}✓${NC} Self-healing is ${BOLD}visible${NC} (you saw the reroute)"
    echo -e "  ${GREEN}✓${NC} Learning is ${BOLD}real-time${NC} (you saw the prior update)"
    echo ""
    pause $PAUSE_LONG

    echo -e "${BOLD}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}   HRM: Safe AI Orchestration for the Real World                       ${NC}"
    echo -e "${BOLD}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════════════
main() {
    case "${1:-full}" in
        problem)   act1_problem ;;
        solution)  act2_solution ;;
        proof)     act3_proof ;;
        diff)      act4_differentiator ;;
        full|*)
            act1_problem
            read -p "Press Enter to continue..."
            act2_solution
            read -p "Press Enter to continue..."
            act3_proof
            read -p "Press Enter to continue..."
            act4_differentiator
            ;;
    esac
}

main "$@"
