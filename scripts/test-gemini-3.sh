#!/bin/bash
# Test runner for Gemini 3 Pro Preview comprehensive test suite

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "================================================="
echo "  Gemini 3 Pro Preview Test Suite Runner"
echo "================================================="
echo ""

# Check for API key
if [ -z "$GEMINI_API_KEY" ]; then
    echo -e "${RED}Error: GEMINI_API_KEY environment variable not set${NC}"
    echo "Please set your Gemini API key:"
    echo "  export GEMINI_API_KEY=your_api_key_here"
    exit 1
fi

# Set model (allow override)
export GEMINI_MODEL="${GEMINI_MODEL:-models/gemini-3-pro-preview}"

echo "Configuration:"
echo "  Model: $GEMINI_MODEL"
echo "  API Key: ${GEMINI_API_KEY:0:10}..."
echo ""

# Parse command line arguments
QUICK_MODE=false
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --quick)
            QUICK_MODE=true
            shift
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --quick       Run only critical tests (faster)"
            echo "  --verbose,-v  Show detailed output"
            echo "  --help,-h     Show this help message"
            echo ""
            echo "Environment variables:"
            echo "  GEMINI_API_KEY  Your Gemini API key (required)"
            echo "  GEMINI_MODEL    Model to test (default: models/gemini-3-pro-preview)"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Quick smoke tests function
run_quick_tests() {
    echo -e "${YELLOW}Running quick smoke tests...${NC}"
    echo ""

    echo "1. Checking model availability..."
    if cargo run -q -p arkavo-gemini --example list_models 2>&1 | grep -q "gemini-3-pro-preview"; then
        echo -e "   ${GREEN}✓ Model available${NC}"
    else
        echo -e "   ${RED}✗ Model not found${NC}"
        return 1
    fi

    echo "2. Testing basic generation..."
    if timeout 30 cargo run -q -p arkavo -- chat --prompt "Say hello" 2>&1 | grep -q -i "hello\|hi"; then
        echo -e "   ${GREEN}✓ Basic generation works${NC}"
    else
        echo -e "   ${RED}✗ Generation failed${NC}"
        return 1
    fi

    echo "3. Testing tool calls..."
    if timeout 60 cargo run -q -p arkavo -- task "List files in src" 2>&1 | grep -q "\.rs"; then
        echo -e "   ${GREEN}✓ Tool calls work${NC}"
    else
        echo -e "   ${RED}✗ Tool calls failed${NC}"
        return 1
    fi

    echo ""
    echo -e "${GREEN}Quick tests passed!${NC}"
}

# Comprehensive tests function
run_comprehensive_tests() {
    echo -e "${YELLOW}Running comprehensive test suite...${NC}"
    echo "This will take approximately 10-15 minutes."
    echo ""

    if [ "$VERBOSE" = true ]; then
        cargo run --example gemini-3-comprehensive-test
    else
        cargo run -q --example gemini-3-comprehensive-test
    fi
}

# Main execution
if [ "$QUICK_MODE" = true ]; then
    run_quick_tests
    exit_code=$?
else
    run_comprehensive_tests
    exit_code=$?
fi

# Check results file
if [ -f "gemini-3-test-results.json" ]; then
    echo ""
    echo -e "${GREEN}Test results saved to: gemini-3-test-results.json${NC}"

    # Show summary if jq is available
    if command -v jq &> /dev/null; then
        echo ""
        echo "Quick summary:"
        passed=$(jq '[.[] | select(.status == "Passed")] | length' gemini-3-test-results.json)
        total=$(jq 'length' gemini-3-test-results.json)
        echo "  Tests passed: $passed / $total"
    fi
fi

exit $exit_code
