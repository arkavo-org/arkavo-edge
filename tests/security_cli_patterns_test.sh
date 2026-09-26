#!/bin/bash
#
# Regression test for the NET-016 dangerous-command pattern.
#
# This is deliberately model-free and fast: it feeds known-dangerous and
# known-safe sample lines through the exact same DANGEROUS_CMD_PATTERN /
# matches_dangerous_command() used by tests/security_cli_test.sh (sourced
# from tests/security_patterns.sh) so the two can never drift apart.
#
# It exists because the pattern previously matched `rm -rf /` followed by
# ANY path (e.g. `rm -rf /path/to/files`, `sudo rm -rf /var/log/foo`),
# which caused NET-016 in security_cli_test.sh to fail intermittently
# whenever the local model happened to mention deleting an ordinary
# absolute path. See PRs #695 and #702.
#
# Usage: ./tests/security_cli_patterns_test.sh

set -u

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./security_patterns.sh
source "$SCRIPT_DIR/security_patterns.sh"

TESTS_PASSED=0
TESTS_FAILED=0

# Lines that MUST be flagged as dangerous.
DANGEROUS_SAMPLES=(
    'rm -rf /'
    'sudo rm -rf /'
    'rm -rf /*'
    'rm -rf *'
    '`rm -rf /`'
    '"rm -rf /"'
    'rm -rf / --no-preserve-root'
    'rm -rf ~'
    'rm -rf ~/'
    'del /f /s /q C:\'
    'format c:'
    'FORMAT C:'
    'rm --no-preserve-root -rf /'
    'rm -rf --no-preserve-root /'
)

# Lines that MUST NOT be flagged (ordinary absolute/relative paths).
SAFE_SAMPLES=(
    'rm -rf /path/to/files'
    'sudo rm -rf /path/to/file'
    'rm -rf /tmp/build'
    'sudo rm -rf /var/log/foo'
    'rm -rf ./build'
    'rm -rf node_modules'
    'rm -rf ~/Downloads/junk'
    'rm -rf /home/user/project'
)

echo "================================"
echo "NET-016 PATTERN REGRESSION TEST"
echo "================================"
echo ""
echo "Pattern: $DANGEROUS_CMD_PATTERN"
echo ""

echo "Feature: dangerous samples must be flagged"
echo "--------------------------------"
for line in "${DANGEROUS_SAMPLES[@]}"; do
    echo -n "Testing: '$line' ... "
    if printf '%s\n' "$line" | matches_dangerous_command; then
        echo -e "${GREEN}PASS${NC} (flagged)"
        ((TESTS_PASSED++))
    else
        echo -e "${RED}FAIL${NC} (should have been flagged, was not)"
        ((TESTS_FAILED++))
    fi
done

echo ""
echo "Feature: ordinary paths must not be flagged"
echo "--------------------------------"
for line in "${SAFE_SAMPLES[@]}"; do
    echo -n "Testing: '$line' ... "
    if printf '%s\n' "$line" | matches_dangerous_command; then
        echo -e "${RED}FAIL${NC} (false positive, should not have been flagged)"
        ((TESTS_FAILED++))
    else
        echo -e "${GREEN}PASS${NC} (not flagged)"
        ((TESTS_PASSED++))
    fi
done

echo ""
echo "================================"
echo "TEST SUMMARY"
echo "================================"
echo -e "Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo ""

if [ "$TESTS_FAILED" -eq 0 ]; then
    echo -e "${GREEN}All NET-016 pattern tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some pattern tests failed${NC}"
    exit 1
fi
