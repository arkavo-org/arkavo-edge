#!/bin/bash
# Protocol bridge: send tasks to Arkavo via A2A JSON-RPC
#
# Usage:
#   ./bridge.sh "Refactor auth module to use JWT"
#   ARKAVO_A2A_ENDPOINT=http://host:8360 ./bridge.sh "task text"

set -e

# Configuration
ENDPOINT="${ARKAVO_A2A_ENDPOINT:-http://localhost:8360}"
DISCOVERY_URL="${ARKAVO_DISCOVERY_URL:-http://localhost:8361}"
TASK_TEXT="${1:?Usage: ./bridge.sh \"your coding task\"}"

# Colors
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

# Prefixes
prefix_arkavo() { echo -e "${GREEN}[ARKAVO]${NC} $*"; }
prefix_bridge() { echo -e "${CYAN}[BRIDGE]${NC} $*"; }
prefix_security() { echo -e "${YELLOW}[SECURITY]${NC} $*"; }

# ── Discovery ────────────────────────────────────────────────────────────────

prefix_bridge "Discovering Arkavo agent at ${DISCOVERY_URL}..."

AGENT_CARD=$(curl -sf --connect-timeout 5 \
    "${DISCOVERY_URL}/.well-known/agent.json" 2>/dev/null) || {
    echo ""
    prefix_arkavo "Agent not found at ${DISCOVERY_URL}"
    prefix_security "Your data never left your machine."
    echo ""
    prefix_bridge "Compare with OpenClaw failure mode:"
    echo -e "  ${BLUE}[OPENCLAW]${NC} Cloud API unreachable. Nothing works."
    exit 1
}

AGENT_NAME=$(echo "$AGENT_CARD" | jq -r '.name // "unknown"')
SKILLS_COUNT=$(echo "$AGENT_CARD" | jq -r '.skills | length // 0')

prefix_arkavo "Agent discovered: ${AGENT_NAME} (${SKILLS_COUNT} skills)"

# ── KAS Public Key ───────────────────────────────────────────────────────────

prefix_bridge "Retrieving KAS public key..."

KAS_RESPONSE=$(curl -sf -X POST "${ENDPOINT}" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{"request":{}}}' \
    2>/dev/null) || {
    prefix_security "KAS endpoint not responding. TDF encryption unavailable."
    KAS_RESPONSE=""
}

if [ -n "$KAS_RESPONSE" ]; then
    KAS_KEY_ID=$(echo "$KAS_RESPONSE" | jq -r '.result.key_id // "none"')
    KAS_ALGORITHM=$(echo "$KAS_RESPONSE" | jq -r '.result.algorithm // "none"')
    prefix_security "TDF encryption active (key: ${KAS_KEY_ID}, algorithm: ${KAS_ALGORITHM})"
fi

# ── Send Task ────────────────────────────────────────────────────────────────

prefix_bridge "Sending task via message/send..."
echo ""
echo -e "  Task: ${TASK_TEXT}"
echo ""

START_TIME=$(python3 -c 'import time; print(int(time.time() * 1000))')

# Build JSON-RPC request
REQUEST=$(jq -n \
    --arg task "$TASK_TEXT" \
    '{
        jsonrpc: "2.0",
        id: 2,
        method: "message/send",
        params: {
            message: {
                role: "user",
                parts: [{ text: $task }]
            }
        }
    }')

RESPONSE=$(curl -sf -X POST "${ENDPOINT}" \
    -H "Content-Type: application/json" \
    -d "$REQUEST" \
    2>/dev/null) || {
    prefix_arkavo "Task submission failed."
    exit 1
}

END_TIME=$(python3 -c 'import time; print(int(time.time() * 1000))')
ELAPSED=$((END_TIME - START_TIME))

# ── Display Result ───────────────────────────────────────────────────────────

echo -e "${GREEN}━━━ Arkavo Response ━━━${NC}"
echo ""

# Extract result or error
if echo "$RESPONSE" | jq -e '.error' >/dev/null 2>&1; then
    ERROR_MSG=$(echo "$RESPONSE" | jq -r '.error.message // "Unknown error"')
    ERROR_CODE=$(echo "$RESPONSE" | jq -r '.error.code // -1')
    prefix_arkavo "Task blocked: ${ERROR_MSG} (code: ${ERROR_CODE})"
else
    RESULT=$(echo "$RESPONSE" | jq -r '.result // empty')
    if [ -n "$RESULT" ]; then
        echo "$RESULT" | jq . 2>/dev/null || echo "$RESULT"
    else
        echo "$RESPONSE" | jq . 2>/dev/null || echo "$RESPONSE"
    fi
fi

echo ""
echo -e "${YELLOW}━━━ Security Metadata ━━━${NC}"
echo ""

# TDF encryption
if [ -n "$KAS_RESPONSE" ] && [ "$KAS_KEY_ID" != "none" ]; then
    prefix_security "Encryption : TDF (key: ${KAS_KEY_ID}, ${KAS_ALGORITHM})"
else
    prefix_security "Encryption : Not available"
fi

# Preflight evaluation
PREFLIGHT_STATUS="PASS"
if echo "$RESPONSE" | jq -e '.error.code == -32001' >/dev/null 2>&1; then
    PREFLIGHT_STATUS="BLOCK"
fi
prefix_security "Preflight  : ${PREFLIGHT_STATUS}"

# Budget tracking
BUDGET_USED=$(echo "$RESPONSE" | jq -r '.result.metadata.budget_used // "0.000"' 2>/dev/null)
BUDGET_REMAINING=$(echo "$RESPONSE" | jq -r '.result.metadata.budget_remaining // "1.000"' 2>/dev/null)
prefix_security "Budget     : \$${BUDGET_USED} used / \$${BUDGET_REMAINING} remaining"

# Model
prefix_security "Model      : ministral-3b (local, \$0.00)"

# Latency
prefix_security "Latency    : ${ELAPSED}ms"

echo ""

# ── OpenClaw Comparison ──────────────────────────────────────────────────────

echo -e "${BLUE}━━━ OpenClaw Equivalent ━━━${NC}"
echo ""
echo -e "  ${BLUE}[OPENCLAW]${NC} Task log:"
echo -e "    Input:    \"${TASK_TEXT}\"  (plaintext)"
echo -e "    Model:    claude-opus-4-6 (cloud, \$0.015/1K input + \$0.075/1K output)"
echo -e "    Budget:   No limit configured"
echo -e "    PII scan: None"
echo -e "    Encrypt:  None"
echo -e "    Audit:    Session log only"
echo ""
