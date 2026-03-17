#!/bin/bash
# Test the secure data plane: encrypt -> stage -> share -> fetch

set -e

SENDER_RPC="${1:-localhost:8080}"
RECEIVER_RPC="${2:-localhost:8082}"
RECEIVER_HTTP="${3:-localhost:8083}"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${CYAN}Testing Secure Data Plane${NC}"
echo "=========================="
echo "Sender RPC:   http://${SENDER_RPC}"
echo "Receiver RPC: http://${RECEIVER_RPC}"
echo ""

# Test 1: Get receiver's KAS public key
echo -e "${YELLOW}Test 1: Get receiver's KAS public key${NC}"
RESPONSE=$(curl -s -X POST "http://${RECEIVER_RPC}" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{"request":{}}}')
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"

if echo "$RESPONSE" | jq -e '.result.public_key' > /dev/null 2>&1; then
    echo -e "${GREEN}[PASS] Receiver KAS public key retrieved${NC}"
    KAS_PUBLIC_KEY=$(echo "$RESPONSE" | jq -r '.result.public_key')
else
    echo -e "${RED}[FAIL] Could not get receiver KAS public key${NC}"
    exit 1
fi
echo ""

# Test 2: Get sender's KAS public key
echo -e "${YELLOW}Test 2: Get sender's KAS public key${NC}"
RESPONSE=$(curl -s -X POST "http://${SENDER_RPC}" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"kas.publicKey","params":{"request":{}}}')
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"

if echo "$RESPONSE" | jq -e '.result.public_key' > /dev/null 2>&1; then
    echo -e "${GREEN}[PASS] Sender KAS public key retrieved${NC}"
else
    echo -e "${RED}[FAIL] Could not get sender KAS public key${NC}"
    exit 1
fi
echo ""

# Test 3: Send a tdf.share offer to the receiver
echo -e "${YELLOW}Test 3: Send tdf.share offer to receiver${NC}"
RESPONSE=$(curl -s -X POST "http://${RECEIVER_RPC}" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\":\"2.0\",
    \"id\":3,
    \"method\":\"tdf.share\",
    \"params\":{
      \"request\":{
        \"ticket\":\"blobaaaa-test-ticket-placeholder\",
        \"content_hash\":\"abc123def456\",
        \"size_bytes\":4096,
        \"policy_attributes\":[\"https://arkavo.net/attr/sensitivity\"],
        \"kas_url\":\"http://${SENDER_RPC}\",
        \"sender_agent_id\":\"data-sender\"
      }
    }
  }")
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"

if echo "$RESPONSE" | jq -e '.result.accepted' > /dev/null 2>&1; then
    OFFER_ID=$(echo "$RESPONSE" | jq -r '.result.offer_id')
    echo -e "${GREEN}[PASS] Share offer accepted (offer_id: ${OFFER_ID})${NC}"
else
    echo -e "${RED}[FAIL] Share offer not accepted${NC}"
fi
echo ""

# Test 4: List pending offers on receiver
echo -e "${YELLOW}Test 4: List pending tdf.offers on receiver${NC}"
RESPONSE=$(curl -s -X POST "http://${RECEIVER_RPC}" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":4,"method":"tdf.offers","params":{"request":{}}}')
echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"

OFFER_COUNT=$(echo "$RESPONSE" | jq -r '.result.offers | length' 2>/dev/null || echo "0")
if [ "$OFFER_COUNT" -gt 0 ]; then
    echo -e "${GREEN}[PASS] Receiver has ${OFFER_COUNT} pending offer(s)${NC}"
else
    echo -e "${RED}[FAIL] No pending offers found${NC}"
fi
echo ""

# Test 5: Check receiver agent card for TDF capabilities
echo -e "${YELLOW}Test 5: Check receiver agent card for TDF skills${NC}"
RESPONSE=$(curl -s "http://${RECEIVER_HTTP}/.well-known/agent.json")
echo "TDF Skills:"
echo "$RESPONSE" | jq '.skills[] | select(.id | startswith("tdf") or startswith("kas"))' 2>/dev/null || echo "No TDF skills found"
echo ""

echo -e "${CYAN}Tests Complete${NC}"
echo ""
echo "Next steps:"
echo "  1. Use 'arkavo tdf encrypt' to encrypt a real file"
echo "  2. Use 'arkavo tdf stage' to stage it to Iroh"
echo "  3. Send the ticket via tdf.share to the receiver"
echo "  4. Receiver fetches and decrypts via KAS rewrap"
