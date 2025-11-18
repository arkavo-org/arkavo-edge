#!/bin/bash
# Direct test runner with real-time feedback
# Usage: GEMINI_API_KEY=xxx ./scripts/run-gemini-3-tests.sh

set -e

if [ -z "${GEMINI_API_KEY}" ]; then
    echo "ERROR: GEMINI_API_KEY not set"
    exit 1
fi

echo "Running Gemini 3 Pro Preview CLI E2E Tests..."
echo ""

# Run tests directly with cargo - output shows in real-time
echo "=== Suite A: Unix Philosophy & Pipes ==="
cargo test --test gemini_3_cli_suite_a -- --nocapture --test-threads=1

echo ""
echo "=== Suite B: MCP Tool Execution ==="
cargo test --test gemini_3_cli_suite_b -- --nocapture --test-threads=1

echo ""
echo "✓ All tests completed"
