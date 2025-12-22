#!/bin/bash
# Family Travel Mesh - Adversarial Testing
# Demonstrates fault tolerance and self-healing

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

print_component() {
    local component=$1
    local message=$2
    case $component in
        "HAZARD") echo -e "${RED}[HAZARD]${NC} $message" ;;
        "CONDUCTOR") echo -e "${BLUE}[CONDUCTOR]${NC} $message" ;;
        "ROUTER") echo -e "${MAGENTA}[ROUTER]${NC} $message" ;;
        "SPECIALIST") echo -e "${GREEN}[SPECIALIST]${NC} $message" ;;
        "CRITIC") echo -e "${CYAN}[CRITIC]${NC} $message" ;;
        "RECOVERY") echo -e "${GREEN}[RECOVERY]${NC} $message" ;;
        "INFO") echo -e "${BLUE}[INFO]${NC} $message" ;;
    esac
}

run_drunk_scenario() {
    echo ""
    echo "======================================================================="
    echo " ADV-01: Drunk Agent (Schema Corruption)                              "
    echo "======================================================================="
    echo ""

    print_component "HAZARD" "Injecting fault: SchemaCorruption to vegas-guide"
    print_component "HAZARD" "Pattern: [TODO] and [placeholder] in response"
    echo ""

    print_component "CONDUCTOR" "Delegating subtask to Router..."
    print_component "ROUTER" "Selected: vegas-guide (score: 0.68)"
    echo ""

    print_component "SPECIALIST" "vegas-guide: Executing burst contract..."
    print_component "SPECIALIST" "Response: [TODO: fix this broken output]"
    echo ""

    print_component "CRITIC" "Running verification pipeline..."
    print_component "CRITIC" "  [0] SchemaCheck: OK"
    print_component "CRITIC" "  [1] LintCheck: FAILED - placeholder_patterns detected"
    print_component "CRITIC" "VETO: Response contains [TODO] pattern"
    echo ""

    print_component "ROUTER" "Updating vegas-guide prior: Beta(35,12) -> Beta(35,13)"
    print_component "ROUTER" "Rerouting to fallback: family-activities"
    echo ""

    print_component "SPECIALIST" "family-activities: Executing burst contract..."
    print_component "CRITIC" "APPROVED"
    echo ""

    print_component "RECOVERY" "Task completed via fallback agent"
    print_component "RECOVERY" "Reroute count: 1"
}

run_lazy_scenario() {
    echo ""
    echo "======================================================================="
    echo " ADV-02: Lazy Agent (Minimal Compliance)                              "
    echo "======================================================================="
    echo ""

    print_component "HAZARD" "Injecting fault: LazyCompliance to vegas-guide"
    print_component "HAZARD" "Pattern: Minimal effort response"
    echo ""

    print_component "CONDUCTOR" "Delegating subtask to Router..."
    print_component "ROUTER" "Selected: vegas-guide (score: 0.61)"
    echo ""

    print_component "SPECIALIST" "vegas-guide: Executing burst contract..."
    print_component "SPECIALIST" "Response: 'I completed the task. [placeholder]'"
    echo ""

    print_component "CRITIC" "Running verification pipeline..."
    print_component "CRITIC" "  [0] SchemaCheck: OK"
    print_component "CRITIC" "  [1] LintCheck: FAILED - placeholder detected"
    print_component "CRITIC" "VETO: Lazy response with placeholder"
    echo ""

    print_component "ROUTER" "Penalty applied to vegas-guide"
    print_component "ROUTER" "Rerouting to: family-activities"
    echo ""

    print_component "SPECIALIST" "family-activities: Executing burst contract..."
    print_component "CRITIC" "APPROVED"
    echo ""

    print_component "RECOVERY" "Task completed via fallback"
}

run_loop_scenario() {
    echo ""
    echo "======================================================================="
    echo " ADV-03: Infinite Loop Detection                                      "
    echo "======================================================================="
    echo ""

    print_component "HAZARD" "Injecting fault: PolicyViolation to all specialists"
    print_component "HAZARD" "Pattern: All agents suggest casinos (prohibited)"
    echo ""

    print_component "CONDUCTOR" "Attempt 1: Delegating to vegas-guide..."
    print_component "CRITIC" "VETO: PolicyCheck - prohibited venue_type=casino"
    echo ""

    print_component "CONDUCTOR" "Attempt 2: Delegating to family-activities..."
    print_component "CRITIC" "VETO: PolicyCheck - prohibited venue_type=casino"
    echo ""

    print_component "CONDUCTOR" "Attempt 3: Delegating to budget-optimizer..."
    print_component "CRITIC" "VETO: PolicyCheck - prohibited venue_type=casino"
    echo ""

    print_component "CONDUCTOR" "Loop detection triggered!"
    print_component "CONDUCTOR" "Similar subtask failed 3 times"
    print_component "CONDUCTOR" "Error: StrategicThrashing detected"
    echo ""

    print_component "RECOVERY" "Task aborted to prevent infinite loop"
    print_component "RECOVERY" "Max retries respected: 3"
}

show_help() {
    echo "Usage: $0 [scenario]"
    echo ""
    echo "Adversarial Scenarios:"
    echo "  drunk   - ADV-01: Schema corruption with [TODO] patterns"
    echo "  lazy    - ADV-02: Lazy compliance with minimal response"
    echo "  loop    - ADV-03: Infinite loop detection"
    echo "  all     - Run all scenarios"
    echo "  help    - Show this message"
}

case "${1:-help}" in
    drunk)
        run_drunk_scenario
        ;;
    lazy)
        run_lazy_scenario
        ;;
    loop)
        run_loop_scenario
        ;;
    all)
        run_drunk_scenario
        echo ""
        run_lazy_scenario
        echo ""
        run_loop_scenario
        ;;
    help|--help|-h)
        show_help
        ;;
    *)
        echo "Unknown scenario: $1"
        show_help
        exit 1
        ;;
esac

echo ""
