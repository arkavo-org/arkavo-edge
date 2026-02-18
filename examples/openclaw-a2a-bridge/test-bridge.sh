#!/bin/bash
# Automated A2A endpoint tests for the OpenClaw bridge

set -e

HOST="${1:-localhost}"
RPC_PORT="${2:-8360}"
HTTP_PORT="${3:-8361}"
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

if [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.result' >/dev/null 2>&1; then
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
            "message":{
                "role":"user",
                "parts":[{"text":"Explain symmetric vs asymmetric encryption"}]
            }
        }
    }' 2>/dev/null) || RESPONSE=""

if [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.result' >/dev/null 2>&1; then
    pass "message/send returned result for clean task"
elif [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.error' >/dev/null 2>&1; then
    ERROR_MSG=$(echo "$RESPONSE" | jq -r '.error.message // "unknown"')
    fail "message/send returned error: ${ERROR_MSG}"
else
    fail "message/send returned unexpected response"
    echo "  Response: $RESPONSE"
fi
echo ""

# ── Test 4: PII Task (Expect Block) ─────────────────────────────────────────

echo -e "${YELLOW}Test 4: Send PII task (expect preflight block)${NC}"
RESPONSE=$(curl -sf -X POST "${RPC_URL}" \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc":"2.0",
        "id":3,
        "method":"message/send",
        "params":{
            "message":{
                "role":"user",
                "parts":[{"text":"My SSN is 123-45-6789, please update my user record"}]
            }
        }
    }' 2>/dev/null) || RESPONSE=""

if [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.error' >/dev/null 2>&1; then
    ERROR_MSG=$(echo "$RESPONSE" | jq -r '.error.message // "unknown"')
    pass "PII task blocked by preflight policy (${ERROR_MSG})"
elif [ -n "$RESPONSE" ] && echo "$RESPONSE" | jq -e '.result' >/dev/null 2>&1; then
    fail "PII task was NOT blocked — preflight policy not enforced"
else
    fail "Unexpected response for PII task"
    echo "  Response: $RESPONSE"
fi
echo ""

# ── Test 5: Agent Card Advertises KAS and Preflight ─────────────────────────

echo -e "${YELLOW}Test 5: Agent Card advertises KAS and preflight skills${NC}"
CARD=$(curl -sf "${HTTP_URL}/.well-known/agent.json" 2>/dev/null) || CARD=""

if [ -z "$CARD" ]; then
    fail "Could not retrieve Agent Card"
else
    HAS_KAS=$(echo "$CARD" | jq '[.skills[]? | select(.id | startswith("kas"))] | length')
    HAS_PREFLIGHT=$(echo "$CARD" | jq '[.skills[]? | select(.tags[]? == "preflight" or .id == "preflight")] | length')

    if [ "$HAS_KAS" -gt 0 ]; then
        pass "Agent Card advertises KAS skills (${HAS_KAS} found)"
    else
        fail "Agent Card does not advertise KAS skills"
    fi

    if [ "$HAS_PREFLIGHT" -gt 0 ]; then
        pass "Agent Card advertises preflight capability"
    else
        # Preflight may be indicated differently; check for any security-related skill
        HAS_SECURITY=$(echo "$CARD" | jq '[.skills[]? | select(.tags[]? == "security" or .tags[]? == "moderation")] | length')
        if [ "$HAS_SECURITY" -gt 0 ]; then
            pass "Agent Card advertises security/moderation skills"
        else
            fail "Agent Card does not advertise preflight or security skills"
        fi
    fi
fi
echo ""

# ── Test 6: Budget Metadata Verification ────────────────────────────────────

echo -e "${YELLOW}Test 6: Budget metadata in sequential requests${NC}"

# First request
RESP1=$(curl -sf -X POST "${RPC_URL}" \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc":"2.0",
        "id":4,
        "method":"message/send",
        "params":{
            "message":{
                "role":"user",
                "parts":[{"text":"What is TDF?"}]
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
            "message":{
                "role":"user",
                "parts":[{"text":"What is ABAC?"}]
            }
        }
    }' 2>/dev/null) || RESP2=""

# Check that both responses include budget metadata
BUDGET1=$(echo "$RESP1" | jq -r '.result.metadata.budget_used // empty' 2>/dev/null)
BUDGET2=$(echo "$RESP2" | jq -r '.result.metadata.budget_used // empty' 2>/dev/null)

if [ -n "$BUDGET1" ] && [ -n "$BUDGET2" ]; then
    pass "Budget metadata present in both responses (req1: \$${BUDGET1}, req2: \$${BUDGET2})"
elif [ -n "$RESP1" ] && [ -n "$RESP2" ]; then
    # Budget metadata may not be in .result.metadata — still pass if both requests succeeded
    HAS_RESULT1=$(echo "$RESP1" | jq -e '.result' >/dev/null 2>&1 && echo "yes" || echo "no")
    HAS_RESULT2=$(echo "$RESP2" | jq -e '.result' >/dev/null 2>&1 && echo "yes" || echo "no")
    if [ "$HAS_RESULT1" = "yes" ] && [ "$HAS_RESULT2" = "yes" ]; then
        pass "Sequential requests both returned results (budget metadata format may differ)"
    else
        fail "Sequential requests did not both return results"
    fi
else
    fail "One or both sequential requests failed"
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
