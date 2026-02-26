#!/bin/bash
# Automated A2A endpoint tests for the OpenClaw bridge

set -e

HOST="${1:-localhost}"
RPC_PORT="${2:-8360}"
HTTP_PORT="${3:-${RPC_PORT}}"
RPC_URL="http://${HOST}:${RPC_PORT}"
HTTP_URL="http://${HOST}:${HTTP_PORT}"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

PASS=0
FAIL=0

pass() {
    echo -e "  ${GREEN}[PASS]${NC} $1"
    PASS=$((PASS + 1))
}

fail() {
    echo -e "  ${RED}[FAIL]${NC} $1"
    FAIL=$((FAIL + 1))
}

echo -e "${CYAN}OpenClaw A2A Bridge Tests${NC}"
echo "========================="
echo "RPC endpoint : ${RPC_URL}"
echo "HTTP endpoint: ${HTTP_URL}"
echo ""

# ── Test 1: Agent Card Discovery ─────────────────────────────────────────────

echo -e "${YELLOW}Test 1: Agent Card discovery${NC}"
RESPONSE=$(curl -sf "${HTTP_URL}/.well-known/agent.json" 2>/dev/null) || RESPONSE=""

if [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.name' >/dev/null 2>&1; then
    AGENT_NAME=$(echo "$RESPONSE" | jq -r '.name')
    pass "Agent Card returned (name: ${AGENT_NAME})"
else
    fail "Agent Card not available at ${HTTP_URL}/.well-known/agent.json"
fi
echo ""

# ── Test 2: KAS Public Key ──────────────────────────────────────────────────

echo -e "${YELLOW}Test 2: KAS public key retrieval${NC}"
RESPONSE=$(curl -sf -X POST "${RPC_URL}" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{"request":{}}}' \
    2>/dev/null) || RESPONSE=""

if [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.result.public_key' >/dev/null 2>&1; then
    KEY_ID=$(echo "$RESPONSE" | jq -r '.result.key_id // "unknown"')
    pass "kas.publicKey returned result (key_id: ${KEY_ID})"
else
    fail "kas.publicKey did not return a result"
    echo "  Response: $RESPONSE"
fi
echo ""

# ── Test 3: Clean Coding Task ───────────────────────────────────────────────

echo -e "${YELLOW}Test 3: Send clean coding task via message/send${NC}"
RESPONSE=$(curl -sf -X POST "${RPC_URL}" \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc":"2.0",
        "id":2,
        "method":"message/send",
        "params":{
            "request":{
                "message":{
                    "parts":[{"type":"text","content":"Explain symmetric vs asymmetric encryption"}]
                }
            }
        }
    }' 2>/dev/null) || RESPONSE=""

if [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.result.task_id' >/dev/null 2>&1; then
    TASK_ID=$(echo "$RESPONSE" | jq -r '.result.task_id')
    STATUS=$(echo "$RESPONSE" | jq -r '.result.status')
    pass "message/send returned task (id: ${TASK_ID}, status: ${STATUS})"
else
    fail "message/send did not return a task result"
    echo "  Response: $RESPONSE"
fi
echo ""

# ── Test 4: PII Task ────────────────────────────────────────────────────────

echo -e "${YELLOW}Test 4: Send PII task via message/send${NC}"
RESPONSE=$(curl -sf -X POST "${RPC_URL}" \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc":"2.0",
        "id":3,
        "method":"message/send",
        "params":{
            "request":{
                "message":{
                    "parts":[{"type":"text","content":"My SSN is 123-45-6789, please update my user record"}]
                }
            }
        }
    }' 2>/dev/null) || RESPONSE=""

# PII may be blocked by preflight (error) or submitted for downstream handling (result)
if [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.error' >/dev/null 2>&1; then
    ERROR_MSG=$(echo "$RESPONSE" | jq -r '.error.message // "unknown"')
    pass "PII task blocked by preflight policy (${ERROR_MSG})"
elif [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.result.task_id' >/dev/null 2>&1; then
    pass "PII task accepted (preflight runs at inference stage)"
else
    fail "Unexpected response for PII task"
    echo "  Response: $RESPONSE"
fi
echo ""

# ── Test 5: Agent Card Advertises KAS Skills ────────────────────────────────

echo -e "${YELLOW}Test 5: Agent Card advertises KAS skills${NC}"
CARD=$(curl -sf "${HTTP_URL}/.well-known/agent.json" 2>/dev/null) || CARD=""

if [ -z "$CARD" ]; then
    fail "Could not retrieve Agent Card"
else
    HAS_KAS=$(echo "$CARD" | jq '[.skills[]? | select(.id | startswith("kas"))] | length')

    if [ "$HAS_KAS" -gt 0 ]; then
        pass "Agent Card advertises KAS skills (${HAS_KAS} found)"
    else
        fail "Agent Card does not advertise KAS skills"
    fi

    # Check for any skills at all
    TOTAL_SKILLS=$(echo "$CARD" | jq '.skills | length')
    if [ "$TOTAL_SKILLS" -gt 0 ]; then
        pass "Agent Card has ${TOTAL_SKILLS} skills total"
    else
        fail "Agent Card has no skills"
    fi
fi
echo ""

# ── Test 6: Sequential Requests ─────────────────────────────────────────────

echo -e "${YELLOW}Test 6: Sequential requests both succeed${NC}"

# First request
RESP1=$(curl -sf -X POST "${RPC_URL}" \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc":"2.0",
        "id":4,
        "method":"message/send",
        "params":{
            "request":{
                "message":{
                    "parts":[{"type":"text","content":"What is TDF?"}]
                }
            }
        }
    }' 2>/dev/null) || RESP1=""

# Second request
RESP2=$(curl -sf -X POST "${RPC_URL}" \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc":"2.0",
        "id":5,
        "method":"message/send",
        "params":{
            "request":{
                "message":{
                    "parts":[{"type":"text","content":"What is ABAC?"}]
                }
            }
        }
    }' 2>/dev/null) || RESP2=""

HAS1=$(echo "$RESP1" | jq -e '.result.task_id' >/dev/null 2>&1 && echo "yes" || echo "no")
HAS2=$(echo "$RESP2" | jq -e '.result.task_id' >/dev/null 2>&1 && echo "yes" || echo "no")

if [ "$HAS1" = "yes" ] && [ "$HAS2" = "yes" ]; then
    ID1=$(echo "$RESP1" | jq -r '.result.task_id')
    ID2=$(echo "$RESP2" | jq -r '.result.task_id')
    if [ "$ID1" != "$ID2" ]; then
        pass "Sequential requests returned distinct task IDs"
    else
        fail "Sequential requests returned same task ID"
    fi
else
    fail "One or both sequential requests failed"
    [ "$HAS1" = "no" ] && echo "  Response 1: $RESP1"
    [ "$HAS2" = "no" ] && echo "  Response 2: $RESP2"
fi
echo ""

# ── Summary ──────────────────────────────────────────────────────────────────

TOTAL=$((PASS + FAIL))
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC} (${TOTAL} total)"
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
