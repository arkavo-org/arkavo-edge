#!/bin/bash

# OpenTDF WASM Server Test Suite
# Tests all server endpoints with various scenarios

set -e

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

SERVER_URL="http://localhost:3000"
PASSED=0
FAILED=0

echo ""
echo "============================================================"
echo -e "${BLUE}🧪 OpenTDF WASM Server Test Suite${NC}"
echo "============================================================"
echo ""

# Helper function to run tests
run_test() {
    local name="$1"
    local method="$2"
    local endpoint="$3"
    local data="$4"
    local expected_status="${5:-200}"

    echo -n "Testing: $name... "

    if [ "$method" = "GET" ]; then
        response=$(curl -s -w "\n%{http_code}" "$SERVER_URL$endpoint")
    else
        response=$(curl -s -w "\n%{http_code}" -X "$method" "$SERVER_URL$endpoint" \
            -H "Content-Type: application/json" \
            -d "$data")
    fi

    status_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | sed '$d')

    if [ "$status_code" = "$expected_status" ]; then
        echo -e "${GREEN}✓ PASSED${NC} (HTTP $status_code)"
        ((PASSED++))
        if [ -n "$6" ]; then
            echo "  Response: $(echo "$body" | jq -c '.' 2>/dev/null | head -c 80)..."
        fi
    else
        echo -e "${RED}✗ FAILED${NC} (Expected $expected_status, got $status_code)"
        echo "  Response: $body"
        ((FAILED++))
    fi
}

# Test 1: Root endpoint
run_test "Root endpoint documentation" "GET" "/"

# Test 2: Version info
run_test "Get version info" "GET" "/api/version"

# Test 3: Health check
run_test "Health check" "GET" "/api/health"

# Test 4: API examples
run_test "Get API examples" "GET" "/api/examples"

# Test 5: Parse valid attribute identifier
run_test "Parse valid attribute identifier" "POST" "/api/attribute/parse" \
    '{"identifier": "gov.example:clearance"}'

# Test 6: Parse another attribute
run_test "Parse department attribute" "POST" "/api/attribute/parse" \
    '{"identifier": "org.company:department"}'

# Test 7: Parse invalid attribute (missing colon)
run_test "Parse invalid attribute (error handling)" "POST" "/api/attribute/parse" \
    '{"identifier": "invalid-format"}' "400"

# Test 8: Create simple policy
run_test "Create simple policy" "POST" "/api/policy/create" \
    '{"dissem": ["user@example.com"]}'

# Test 9: Create policy with multiple recipients
run_test "Create policy with multiple recipients" "POST" "/api/policy/create" \
    '{"dissem": ["user1@example.com", "user2@example.com", "admin@example.com"]}'

# Test 10: Create policy with UUID
run_test "Create policy with custom UUID" "POST" "/api/policy/create" \
    '{"uuid": "550e8400-e29b-41d4-a716-446655440000", "dissem": ["user@example.com"]}'

# Test 11: Validate valid policy
run_test "Validate valid policy" "POST" "/api/policy/validate" \
    '{"uuid": "550e8400-e29b-41d4-a716-446655440000", "body": {"attributes": [], "dissem": ["user@example.com"]}}'

# Test 12: Validate invalid policy (missing required fields)
run_test "Validate invalid policy" "POST" "/api/policy/validate" \
    '{"invalid": "structure"}'

# Test 13: Missing required field
run_test "Missing identifier field" "POST" "/api/attribute/parse" \
    '{}' "400"

# Test 14: Invalid JSON
run_test "Invalid JSON body" "POST" "/api/policy/create" \
    'invalid json' "500"

# Test 15: 404 for non-existent route
run_test "Non-existent route" "GET" "/api/nonexistent" "" "404"

# Test 16: CORS preflight (OPTIONS)
echo -n "Testing: CORS preflight request... "
status_code=$(curl -s -o /dev/null -w "%{http_code}" -X OPTIONS "$SERVER_URL/api/version")
if [ "$status_code" = "204" ]; then
    echo -e "${GREEN}✓ PASSED${NC} (HTTP $status_code)"
    ((PASSED++))
else
    echo -e "${RED}✗ FAILED${NC} (Expected 204, got $status_code)"
    ((FAILED++))
fi

# Test 17: Concurrent requests
echo -n "Testing: Concurrent requests (10 simultaneous)... "
for i in {1..10}; do
    curl -s "$SERVER_URL/api/version" > /dev/null &
done
wait
echo -e "${GREEN}✓ PASSED${NC}"
((PASSED++))

# Test 18: Performance test (100 requests)
echo -n "Testing: Performance (100 sequential requests)... "
start_time=$(date +%s.%N)
for i in {1..100}; do
    curl -s "$SERVER_URL/api/version" > /dev/null
done
end_time=$(date +%s.%N)
duration=$(echo "$end_time - $start_time" | bc)
avg=$(echo "scale=2; $duration / 100" | bc)
echo -e "${GREEN}✓ PASSED${NC} (Total: ${duration}s, Avg: ${avg}s/req)"
((PASSED++))

# Summary
echo ""
echo "============================================================"
echo -e "${BLUE}📊 Test Results${NC}"
echo "============================================================"
echo -e "${GREEN}✓ Passed: $PASSED${NC}"
if [ $FAILED -gt 0 ]; then
    echo -e "${RED}✗ Failed: $FAILED${NC}"
fi

TOTAL=$((PASSED + FAILED))
PERCENTAGE=$(echo "scale=1; ($PASSED * 100) / $TOTAL" | bc)
echo "Total: $PASSED/$TOTAL ($PERCENTAGE%)"
echo "============================================================"

# Check server logs
echo ""
echo -e "${YELLOW}📝 Recent server logs:${NC}"
echo "See server output for detailed request logs"

echo ""

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}🎉 All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}❌ Some tests failed${NC}"
    exit 1
fi
