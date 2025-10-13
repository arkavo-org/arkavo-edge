#!/bin/bash
set -e

# UI Generator Integration Test Runner
# This script runs all integration tests with proper setup

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🧪 UI Generator Integration Test Suite"
echo "========================================"

# Check for Gemini API key
if [ -z "$GEMINI_API_KEY" ]; then
    echo "❌ Error: GEMINI_API_KEY environment variable not set"
    echo ""
    echo "To run integration tests, you need a Gemini API key:"
    echo "  export GEMINI_API_KEY='your-api-key-here'"
    echo ""
    echo "Get your API key at: https://ai.google.dev/"
    exit 1
fi

echo "✅ Gemini API key detected"

# Create output directory
OUTPUT_DIR="$SCRIPT_DIR/../../target/test-output"
mkdir -p "$OUTPUT_DIR"
echo "📁 Screenshots will be saved to: $OUTPUT_DIR"
echo ""

# Parse command line arguments
TEST_NAME="${1:-}"

if [ -z "$TEST_NAME" ]; then
    # Run all tests
    echo "🚀 Running all integration tests..."
    echo ""

    cargo test --test integration_test -- --ignored --nocapture

else
    # Run specific test
    echo "🚀 Running test: $TEST_NAME"
    echo ""

    cargo test --test integration_test "$TEST_NAME" -- --ignored --nocapture
fi

# Check if tests produced screenshots
SCREENSHOT_COUNT=$(ls -1 "$OUTPUT_DIR"/*.png 2>/dev/null | wc -l)

echo ""
echo "========================================"
echo "✅ Tests complete!"
echo "📸 Generated $SCREENSHOT_COUNT screenshots"
echo ""
echo "View results:"
echo "  open $OUTPUT_DIR"
echo ""
echo "Available tests:"
echo "  - test_calculator_ui_generation"
echo "  - test_dashboard_ui_generation"
echo "  - test_form_ui_generation"
echo "  - test_bank_portfolio_ui"
echo "  - test_incremental_updates"
echo ""
echo "Run specific test:"
echo "  ./run_integration_tests.sh test_calculator_ui_generation"
