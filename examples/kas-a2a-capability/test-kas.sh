#!/bin/bash
# Test KAS A2A JSON-RPC methods

set -e

HOST="${1:-localhost}"
PORT="${2:-8080}"
BASE_URL="http://${HOST}:${PORT}"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${CYAN}Testing KAS A2A Methods${NC}"
echo "========================"
echo "Endpoint: ${BASE_URL}"
echo ""

# Test 1: kas.publicKey
echo -e "${YELLOW}Test 1: kas.publicKey${NC}"
echo "Request:"
cat <<EOF
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "kas.publicKey",
  "params": {}
}
EOF
echo ""
echo "Response:"
RESPONSE=$(curl -s -X POST "${BASE_URL}" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{}}')
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
echo ""

# Check if successful
if echo "$RESPONSE" | jq -e '.result' > /dev/null 2>&1; then
    echo -e "${GREEN}[PASS] kas.publicKey returned result${NC}"
else
    echo -e "${RED}[FAIL] kas.publicKey returned error${NC}"
fi
echo ""

# Test 2: kas.publicKey with algorithm
echo -e "${YELLOW}Test 2: kas.publicKey with algorithm${NC}"
echo "Request:"
cat <<EOF
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "kas.publicKey",
  "params": {"algorithm": "RSA-OAEP"}
}
EOF
echo ""
echo "Response:"
RESPONSE=$(curl -s -X POST "${BASE_URL}" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"kas.publicKey","params":{"algorithm":"RSA-OAEP"}}')
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
echo ""

# Test 3: kas.rewrap (expected to fail without handler)
echo -e "${YELLOW}Test 3: kas.rewrap (expecting error without handler)${NC}"
echo "Request:"
cat <<EOF
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "kas.rewrap",
  "params": {
    "wrapped_key": "dGVzdC13cmFwcGVkLWtleQ==",
    "policy_binding": {"alg": "HS256", "hash": "dGVzdC1oYXNo"},
    "policy": "eyJhdHRyaWJ1dGVzIjpbXX0=",
    "delegation_token": "{}",
    "client_public_key": "-----BEGIN PUBLIC KEY-----\ntest\n-----END PUBLIC KEY-----"
  }
}
EOF
echo ""
echo "Response:"
RESPONSE=$(curl -s -X POST "${BASE_URL}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":3,
    "method":"kas.rewrap",
    "params":{
      "wrapped_key":"dGVzdC13cmFwcGVkLWtleQ==",
      "policy_binding":{"alg":"HS256","hash":"dGVzdC1oYXNo"},
      "policy":"eyJhdHRyaWJ1dGVzIjpbXX0=",
      "delegation_token":"{}",
      "client_public_key":"-----BEGIN PUBLIC KEY-----\ntest\n-----END PUBLIC KEY-----"
    }
  }')
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
echo ""

# Check for expected error
if echo "$RESPONSE" | jq -e '.error.code == -32603' > /dev/null 2>&1; then
    echo -e "${GREEN}[PASS] kas.rewrap returned expected error (KAS not configured)${NC}"
else
    echo -e "${YELLOW}[INFO] kas.rewrap returned unexpected response${NC}"
fi
echo ""

# Test 4: Check Agent Card
HTTP_PORT=$((PORT + 1))
echo -e "${YELLOW}Test 4: Check Agent Card for KAS skills${NC}"
echo "Fetching: http://${HOST}:${HTTP_PORT}/.well-known/agent.json"
echo ""
RESPONSE=$(curl -s "http://${HOST}:${HTTP_PORT}/.well-known/agent.json")
echo "KAS Skills:"
echo "$RESPONSE" | jq '.skills[] | select(.id | startswith("kas"))' 2>/dev/null || echo "No KAS skills found (feature may not be enabled)"
echo ""

echo -e "${CYAN}Tests Complete${NC}"
