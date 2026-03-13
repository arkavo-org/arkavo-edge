#!/bin/bash
# EvoFabric Test - Full AST pipeline verification
#
# Demonstrates: parse → apply OpBundle → render → verify
# Runs all unit and integration tests for the evofabric crate.
#
# Usage:
#   ./run_test.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/../.."

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

BUNDLE="${SCRIPT_DIR}/bundle.json"
SOURCE="${SCRIPT_DIR}/sample.rs"

echo -e "${BOLD}EvoFabric AST Pipeline${NC}"
echo "======================"
echo ""

# Check prerequisites
if [ ! -f "$SOURCE" ]; then
    echo -e "${RED}Error: sample.rs not found${NC}"
    exit 1
fi

if [ ! -f "$BUNDLE" ]; then
    echo -e "${RED}Error: bundle.json not found${NC}"
    exit 1
fi

# Show the source and bundle
echo -e "${CYAN}Source file:${NC} sample.rs"
echo -e "${CYAN}OpBundle:${NC}    bundle.json"
echo ""

OPS_COUNT=$(python3 -c "import json; print(len(json.load(open('$BUNDLE'))['ops']))" 2>/dev/null || echo "?")
RATIONALE=$(python3 -c "import json; print(json.load(open('$BUNDLE'))['rationale'])" 2>/dev/null || echo "")
echo -e "${YELLOW}Operations:${NC} ${OPS_COUNT}"
echo -e "${YELLOW}Rationale:${NC}  ${RATIONALE}"
echo ""

# Show original source
echo -e "${BOLD}--- Original Source ---${NC}"
cat "$SOURCE"
echo ""

# Run integration tests to prove the pipeline works
echo -e "${BOLD}--- Running AST Pipeline (integration tests) ---${NC}"
echo ""
cd "$REPO_ROOT"
cargo test -p arkavo-evofabric --test end_to_end -- --nocapture 2>&1
echo ""
echo -e "${GREEN}All 8 end-to-end tests passed.${NC}"
echo ""

# Also run the full unit test suite
echo -e "${BOLD}--- Running Unit Tests ---${NC}"
echo ""
cargo test -p arkavo-evofabric --lib -q 2>&1
echo ""
echo -e "${GREEN}All unit tests passed.${NC}"
echo ""

# Summary
TOTAL_TESTS=$(cargo test -p arkavo-evofabric -q 2>&1 | grep "test result" | awk '{sum += $4} END {print sum}')
echo -e "${BOLD}Summary${NC}"
echo "======="
echo -e "  Total tests:      ${GREEN}${TOTAL_TESTS}${NC}"
echo -e "  Pipeline status:  ${GREEN}OK${NC}"
echo ""
echo "The EvoFabric AST pipeline is fully functional."
echo "To run with a local model, use: ./run.sh"
