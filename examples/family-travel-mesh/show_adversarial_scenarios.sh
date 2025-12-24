#!/bin/bash
# Family Travel Mesh - Adversarial Test Scenarios
# Documents expected HRM fault tolerance behavior
# Actual tests run via: cargo test -p golden-dataset

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

show_scenario() {
    local file=$1
    echo ""
    echo "═══════════════════════════════════════════════════════════════════════"
    cat "$file" | python3 -m json.tool 2>/dev/null || cat "$file"
    echo "═══════════════════════════════════════════════════════════════════════"
    echo ""
}

case "${1:-help}" in
    drunk)
        echo "ADV-01: Drunk Agent (Schema Corruption)"
        echo "Tests: Critic LintCheck catches [TODO]/[placeholder] patterns"
        echo "Expected: Fallback to reliable agent, task succeeds"
        show_scenario "$SCRIPT_DIR/scenarios/adversarial/adv_drunk.json"
        echo "Run actual test: cargo test -p golden-dataset test_adv_01"
        ;;
    lazy)
        echo "ADV-02: Lazy Agent (Minimal Compliance)"
        echo "Tests: Critic LintCheck catches lazy placeholder responses"
        echo "Expected: Veto, reroute, fallback succeeds"
        show_scenario "$SCRIPT_DIR/scenarios/adversarial/adv_lazy.json"
        echo "Run actual test: cargo test -p golden-dataset test_adv_02"
        ;;
    loop)
        echo "ADV-03: Infinite Loop Detection"
        echo "Tests: Conductor StrategicThrashing detection"
        echo "Expected: Task aborts after max retries"
        show_scenario "$SCRIPT_DIR/scenarios/adversarial/adv_loop.json"
        echo "Run actual test: cargo test -p golden-dataset test_adv_03"
        ;;
    all)
        $0 drunk
        $0 lazy
        $0 loop
        ;;
    help|--help|-h|*)
        echo "Usage: $0 [drunk|lazy|loop|all]"
        echo ""
        echo "Shows adversarial scenario definitions."
        echo "Actual fault injection tests run via cargo test."
        echo ""
        echo "Scenarios:"
        echo "  drunk - ADV-01: Schema corruption ([TODO] patterns)"
        echo "  lazy  - ADV-02: Lazy compliance (placeholder text)"
        echo "  loop  - ADV-03: Infinite loop detection"
        echo "  all   - Show all scenarios"
        ;;
esac
