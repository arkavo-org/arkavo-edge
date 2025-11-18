#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
RESULTS_FILE="${PROJECT_ROOT}/docs/gemini-3-e2e-cli-results.md"

echo "=================================================="
echo "  Gemini 3 Pro Preview CLI E2E Test Runner"
echo "=================================================="
echo ""

if [ -z "${GEMINI_API_KEY}" ]; then
    echo "ERROR: GEMINI_API_KEY environment variable is not set"
    echo ""
    echo "Usage:"
    echo "  export GEMINI_API_KEY=your_api_key_here"
    echo "  ./scripts/test-gemini-3-cli.sh"
    echo ""
    exit 1
fi

echo "✓ GEMINI_API_KEY is set"
echo "  Model: models/gemini-3-pro-preview"
echo ""

cd "${PROJECT_ROOT}"

echo "Building arkavo binary..."
cargo build 2>&1 | tail -3
echo ""

echo "Running CLI Test Suite A (Unix Philosophy & Pipes)..."
echo "  - CLI-01: STDIN Streaming"
echo "  - CLI-02: STDOUT Redirection"
echo "  - CLI-03: Interrupt Handling"
echo "  - CLI-04: Environment Config"
echo ""

SUITE_A_START=$(date +%s)
SUITE_A_OUTPUT=$(cargo test --test gemini_3_cli_suite_a -- --nocapture 2>&1)
SUITE_A_EXIT=$?
SUITE_A_END=$(date +%s)
SUITE_A_DURATION=$((SUITE_A_END - SUITE_A_START))

if [ $SUITE_A_EXIT -eq 0 ]; then
    echo "✓ Suite A PASSED (${SUITE_A_DURATION}s)"
else
    echo "✗ Suite A FAILED (${SUITE_A_DURATION}s)"
    echo "${SUITE_A_OUTPUT}"
fi
echo ""

echo "Running CLI Test Suite B (MCP Tool Execution)..."
echo "  - CLI-05: Filesystem Access"
echo "  - CLI-06: Git Integration"
echo ""

SUITE_B_START=$(date +%s)
SUITE_B_OUTPUT=$(cargo test --test gemini_3_cli_suite_b -- --nocapture 2>&1)
SUITE_B_EXIT=$?
SUITE_B_END=$(date +%s)
SUITE_B_DURATION=$((SUITE_B_END - SUITE_B_START))

if [ $SUITE_B_EXIT -eq 0 ]; then
    echo "✓ Suite B PASSED (${SUITE_B_DURATION}s)"
else
    echo "✗ Suite B FAILED (${SUITE_B_DURATION}s)"
    echo "${SUITE_B_OUTPUT}"
fi
echo ""

echo "=================================================="
echo "  Test Results Summary"
echo "=================================================="
echo ""

TOTAL_DURATION=$((SUITE_A_DURATION + SUITE_B_DURATION))

if [ $SUITE_A_EXIT -eq 0 ] && [ $SUITE_B_EXIT -eq 0 ]; then
    echo "✓ ALL TESTS PASSED"
    echo "  Total Duration: ${TOTAL_DURATION}s"
    echo ""
    OVERALL_RESULT="PASS"
else
    echo "✗ SOME TESTS FAILED"
    echo "  Suite A: $([ $SUITE_A_EXIT -eq 0 ] && echo "PASS" || echo "FAIL")"
    echo "  Suite B: $([ $SUITE_B_EXIT -eq 0 ] && echo "PASS" || echo "FAIL")"
    echo "  Total Duration: ${TOTAL_DURATION}s"
    echo ""
    OVERALL_RESULT="FAIL"
fi

echo "Writing results to ${RESULTS_FILE}..."
mkdir -p "$(dirname "${RESULTS_FILE}")"

cat > "${RESULTS_FILE}" <<EOF
# Gemini 3 Pro Preview CLI E2E Test Results

**Date**: $(date +"%Y-%m-%d %H:%M:%S")
**Model**: models/gemini-3-pro-preview
**Overall Result**: ${OVERALL_RESULT}
**Total Duration**: ${TOTAL_DURATION}s

## Test Suite A: Unix Philosophy & Pipes

**Duration**: ${SUITE_A_DURATION}s
**Result**: $([ $SUITE_A_EXIT -eq 0 ] && echo "✓ PASS" || echo "✗ FAIL")

### Tests:
- **CLI-01**: STDIN Streaming - $(echo "${SUITE_A_OUTPUT}" | grep -q "CLI-01 PASS" && echo "✓ PASS" || echo "✗ FAIL")
- **CLI-02**: STDOUT Redirection - $(echo "${SUITE_A_OUTPUT}" | grep -q "CLI-02 PASS" && echo "✓ PASS" || echo "✗ FAIL")
- **CLI-03**: Interrupt Handling - $(echo "${SUITE_A_OUTPUT}" | grep -q "CLI-03 PASS" && echo "✓ PASS" || echo "✗ FAIL")
- **CLI-04**: Environment Config - $(echo "${SUITE_A_OUTPUT}" | grep -q "CLI-04 PASS" && echo "✓ PASS" || echo "✗ FAIL")

### Output:
\`\`\`
${SUITE_A_OUTPUT}
\`\`\`

## Test Suite B: MCP Tool Execution

**Duration**: ${SUITE_B_DURATION}s
**Result**: $([ $SUITE_B_EXIT -eq 0 ] && echo "✓ PASS" || echo "✗ FAIL")

### Tests:
- **CLI-05**: Filesystem Access - $(echo "${SUITE_B_OUTPUT}" | grep -q "CLI-05a PASS" && echo "✓ PASS" || echo "✗ FAIL")
- **CLI-06**: Git Integration - $(echo "${SUITE_B_OUTPUT}" | grep -q "CLI-06 PASS" && echo "✓ PASS" || echo "✗ FAIL")

### Output:
\`\`\`
${SUITE_B_OUTPUT}
\`\`\`

## Summary

Total tests executed: 6
- Suite A: 4 tests
- Suite B: 2 tests

**Key Metrics:**
- TTFT (Time To First Token): Measured in CLI-01
- Process termination time: Measured in CLI-03
- Total test execution time: ${TOTAL_DURATION}s

## Related Issues

- [#358](https://github.com/arkavo-org/arkavo-edge/issues/358) - Gemini 3 Pro Preview: Multi-Environment E2E Test Plan
EOF

echo "✓ Results saved to ${RESULTS_FILE}"
echo ""

if [ "${OVERALL_RESULT}" = "PASS" ]; then
    exit 0
else
    exit 1
fi
