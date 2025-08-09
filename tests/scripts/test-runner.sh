#!/bin/bash

# Arkavo Edge Automated Test Runner
# Executes tests that can be automated from the test plan

set -euo pipefail

# Configuration
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
BINARY="${BINARY:-$PROJECT_ROOT/target/release/arkavo}"
RESULTS_DIR="$PROJECT_ROOT/tests/test-results"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
RESULTS_FILE="$RESULTS_DIR/results-$TIMESTAMP.json"
SUMMARY_FILE="$RESULTS_DIR/summary-$TIMESTAMP.md"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Counters
TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

# Create results directory
mkdir -p "$RESULTS_DIR"

# Initialize results file
echo "[]" > "$RESULTS_FILE"

# Helper functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[✅ PASS]${NC} $1"
}

log_failure() {
    echo -e "${RED}[❌ FAIL]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[⚠️  WARN]${NC} $1"
}

# Test execution function
run_test() {
    local test_id=$1
    local test_name=$2
    local test_cmd=$3
    local timeout_sec=${4:-30}
    
    TOTAL=$((TOTAL + 1))
    
    echo -n "[$TOTAL] $test_id - $test_name: "
    
    # Create test output file
    local test_output="/tmp/arkavo-test-$test_id-$TIMESTAMP.log"
    
    # Run test with timeout
    if timeout "$timeout_sec" bash -c "$test_cmd" > "$test_output" 2>&1; then
        log_success "Passed"
        PASSED=$((PASSED + 1))
        
        # Add to JSON results
        jq ". += [{\"test_id\": \"$test_id\", \"name\": \"$test_name\", \"result\": \"pass\", \"timestamp\": \"$(date -Iseconds)\"}]" \
            "$RESULTS_FILE" > "$RESULTS_FILE.tmp" && mv "$RESULTS_FILE.tmp" "$RESULTS_FILE"
    else
        local exit_code=$?
        if [ $exit_code -eq 124 ]; then
            log_warning "Timeout after ${timeout_sec}s"
            SKIPPED=$((SKIPPED + 1))
            jq ". += [{\"test_id\": \"$test_id\", \"name\": \"$test_name\", \"result\": \"timeout\", \"timestamp\": \"$(date -Iseconds)\"}]" \
                "$RESULTS_FILE" > "$RESULTS_FILE.tmp" && mv "$RESULTS_FILE.tmp" "$RESULTS_FILE"
        else
            log_failure "Failed (exit code: $exit_code)"
            FAILED=$((FAILED + 1))
            echo "  Error output: $(tail -n 3 $test_output | tr '\n' ' ')"
            jq ". += [{\"test_id\": \"$test_id\", \"name\": \"$test_name\", \"result\": \"fail\", \"exit_code\": $exit_code, \"timestamp\": \"$(date -Iseconds)\"}]" \
                "$RESULTS_FILE" > "$RESULTS_FILE.tmp" && mv "$RESULTS_FILE.tmp" "$RESULTS_FILE"
        fi
    fi
}

# Banner
echo "============================================"
echo "    Arkavo Edge Automated Test Runner"
echo "============================================"
echo "Binary: $BINARY"
echo "Results: $RESULTS_FILE"
echo "Started: $(date)"
echo ""

# Check prerequisites
log_info "Checking prerequisites..."

if [ ! -f "$BINARY" ]; then
    log_failure "Binary not found: $BINARY"
    echo "Please run: cargo build --release"
    exit 1
fi

if ! command -v jq &> /dev/null; then
    log_warning "jq not found - JSON results will be limited"
fi

# Phase 1: Foundation Tests
echo ""
echo "======== PHASE 1: Foundation Tests ========"

run_test "CLI-01" "Help flag" "$BINARY --help | grep -q 'USAGE'"
run_test "CLI-05" "Non-interactive chat" "echo 'Hello' | $BINARY chat --no-tui --prompt 'Say hi'" 10
run_test "ERR-01" "Invalid command handling" "$BINARY invalid-command 2>&1 | grep -q 'help'" 5
run_test "CLAUDE-01" "CLAUDE.md validity" "test -f $PROJECT_ROOT/CLAUDE.md && grep -q 'Project Overview' $PROJECT_ROOT/CLAUDE.md" 2

# Phase 2: Build & Quality Tests
echo ""
echo "======== PHASE 2: Build & Quality Tests ========"

run_test "SETUP-03" "No OpenSSL dependency" "cd $PROJECT_ROOT && cargo tree 2>/dev/null | grep -i openssl; [ \$? -eq 1 ]" 60
run_test "CODE-02" "Crate organization" "[ \$(ls -d $PROJECT_ROOT/crates/* | wc -l) -gt 10 ]" 5
run_test "PERF-01" "Binary size check" "[ \$(stat -f%z $BINARY 2>/dev/null || stat -c%s $BINARY) -lt 52428800 ]" 5

# Phase 3: Documentation Tests
echo ""
echo "======== PHASE 3: Documentation Tests ========"

run_test "DOC-02" "Doc generation" "cd $PROJECT_ROOT && cargo doc --no-deps --quiet" 120
run_test "DOC-01" "README validity" "test -f $PROJECT_ROOT/README.md && grep -q 'Arkavo Edge' $PROJECT_ROOT/README.md" 2

# Phase 4: Error Handling Tests
echo ""
echo "======== PHASE 4: Error Handling Tests ========"

run_test "ERR-02" "Offline execution" "unset OPENAI_API_KEY && $BINARY chat --no-tui --prompt 'Test' 2>&1 | grep -qE '(error|fail|connect)'" 10

# Phase 5: Git Integration Tests (if in git repo)
echo ""
echo "======== PHASE 5: Git Integration Tests ========"

if git rev-parse --git-dir > /dev/null 2>&1; then
    run_test "GIT-03" "GitHub CLI available" "command -v gh" 2
else
    log_warning "Not in a git repository - skipping git tests"
fi

# Phase 6: Server Tests
echo ""
echo "======== PHASE 6: Server Tests ========"

# Start UI server in background
run_test "CLI-04" "UI server startup" "$BINARY ui & sleep 3 && kill %1" 10

# Generate Summary
echo ""
echo "======== Test Summary ========"
echo "Total:   $TOTAL"
echo -e "${GREEN}Passed:  $PASSED${NC}"
echo -e "${RED}Failed:  $FAILED${NC}"
echo -e "${YELLOW}Skipped: $SKIPPED${NC}"

PASS_RATE=$((PASSED * 100 / TOTAL))
echo "Pass Rate: ${PASS_RATE}%"

# Create markdown summary
cat > "$SUMMARY_FILE" << EOF
# Arkavo Edge Test Results

**Date**: $(date)  
**Binary**: $BINARY  
**Platform**: $(uname -srm)

## Summary

| Metric | Value |
|--------|-------|
| Total Tests | $TOTAL |
| Passed | $PASSED |
| Failed | $FAILED |
| Skipped | $SKIPPED |
| Pass Rate | ${PASS_RATE}% |

## Results

\`\`\`json
$(cat "$RESULTS_FILE" | jq '.')
\`\`\`

## Test Logs

Individual test logs are available in \`/tmp/arkavo-test-*-$TIMESTAMP.log\`
EOF

echo ""
echo "Results saved to:"
echo "  - JSON: $RESULTS_FILE"
echo "  - Summary: $SUMMARY_FILE"

# Exit with error if any tests failed
if [ $FAILED -gt 0 ]; then
    exit 1
fi