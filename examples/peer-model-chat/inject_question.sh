#!/bin/bash
# Inject a question that creates a "knowledge gap"
#
# This demonstrates peer collaboration:
# - Ask Alice (general chat model) a code question
# - Alice recognizes this is outside her expertise
# - Alice queries Bob (code model) via A2A
# - Bob provides the structured answer
# - Alice relays the response

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Colors for output
BLUE='\033[0;34m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo ""
echo "  ━━━━━━ INJECTING KNOWLEDGE GAP QUESTION ━━━━━━"
echo ""

# Read ports from files (created by launch_chat.sh)
if [ -f .alice.port ]; then
    ALICE_PORT=$(cat .alice.port)
    ALICE_URL="http://localhost:$ALICE_PORT"
else
    echo -e "${RED}[ERROR ]${NC} Alice port file not found. Run ./launch_chat.sh first"
    exit 1
fi

if [ -f .bob.port ]; then
    BOB_PORT=$(cat .bob.port)
    BOB_URL="http://localhost:$BOB_PORT"
else
    echo -e "${RED}[ERROR ]${NC} Bob port file not found. Run ./launch_chat.sh first"
    exit 1
fi

echo -e "${BLUE}[INFO  ]${NC} Alice endpoint: $ALICE_URL"
echo -e "${BLUE}[INFO  ]${NC} Bob endpoint: $BOB_URL"
echo ""

# Check if agents are running
check_agent() {
    local url=$1
    curl -s -X POST "$url/rpc" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"rpc.discover","params":{},"id":1}' \
        2>/dev/null | grep -q '"result"'
}

if ! check_agent "$ALICE_URL"; then
    echo -e "${RED}[ERROR ]${NC} Alice is not responding at $ALICE_URL"
    echo -e "${RED}[ERROR ]${NC} Start the agents first with ./launch_chat.sh"
    exit 1
fi

if ! check_agent "$BOB_URL"; then
    echo -e "${RED}[ERROR ]${NC} Bob is not responding at $BOB_URL"
    echo -e "${RED}[ERROR ]${NC} Start the agents first with ./launch_chat.sh"
    exit 1
fi

# Function to send message and get task ID
send_message() {
    local url=$1
    local message=$2

    curl -s -X POST "$url/rpc" \
        -H "Content-Type: application/json" \
        -d "{
            \"jsonrpc\": \"2.0\",
            \"method\": \"message/send\",
            \"params\": {
                \"request\": {
                    \"message\": {
                        \"parts\": [{\"type\": \"text\", \"content\": \"$message\"}]
                    }
                }
            },
            \"id\": 1
        }"
}

# Function to poll task status
poll_task() {
    local url=$1
    local task_id=$2
    local max_wait=$3
    local waited=0

    while [ $waited -lt $max_wait ]; do
        local response=$(curl -s -X POST "$url/rpc" \
            -H "Content-Type: application/json" \
            -d "{
                \"jsonrpc\": \"2.0\",
                \"method\": \"tasks/get\",
                \"params\": {
                    \"request\": {
                        \"task_id\": \"$task_id\"
                    }
                },
                \"id\": 1
            }")

        local status=$(echo "$response" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('result',{}).get('status',''))" 2>/dev/null)

        if [ "$status" = "completed" ]; then
            echo "$response"
            return 0
        elif [ "$status" = "failed" ]; then
            echo "$response"
            return 1
        fi

        sleep 2
        waited=$((waited + 2))
        echo -ne "\r  Waiting for response... ${waited}s"
    done
    echo ""
    echo "$response"
    return 2
}

# Scenario 1: Ask Alice a code question (she should ask Bob)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo -e "${BLUE}[INJECT]${NC} Asking Alice a code question..."
echo ""
echo -e "${YELLOW}Question:${NC} \"Write a Rust function that implements binary search.\""
echo ""

RESPONSE=$(send_message "$ALICE_URL" "Write a Rust function that implements binary search. I need structured code output.")
TASK_ID=$(echo "$RESPONSE" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('result',{}).get('task_id',''))" 2>/dev/null)

if [ -n "$TASK_ID" ]; then
    echo -e "${GREEN}[ALICE ]${NC} Task submitted: $TASK_ID"
    echo ""

    # Poll for result
    RESULT=$(poll_task "$ALICE_URL" "$TASK_ID" 60)
    echo ""

    # Extract and display the response content
    CONTENT=$(echo "$RESULT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    parts = d.get('result', {}).get('result', {}).get('parts', [])
    for part in parts:
        if part.get('type') == 'text':
            print(part.get('content', ''))
except:
    pass
" 2>/dev/null)

    if [ -n "$CONTENT" ]; then
        echo -e "${GREEN}[ALICE ]${NC} Response:"
        echo "─────────────────────────────────────────────────"
        echo "$CONTENT" | head -50
        echo "─────────────────────────────────────────────────"
    fi
else
    echo -e "${RED}[ERROR ]${NC} Failed to submit task"
    echo "$RESPONSE"
fi

echo ""
echo "  Watch logs/alice.log for peer collaboration details"
echo ""

# Scenario 2: Ask Bob a creative question (he should ask Alice)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo -e "${BLUE}[INJECT]${NC} Asking Bob a creative writing question..."
echo ""
echo -e "${YELLOW}Question:${NC} \"Write a short poem about programming.\""
echo ""

RESPONSE=$(send_message "$BOB_URL" "Write a short creative poem about the joy of programming. Be expressive and emotional.")
TASK_ID=$(echo "$RESPONSE" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('result',{}).get('task_id',''))" 2>/dev/null)

if [ -n "$TASK_ID" ]; then
    echo -e "${GREEN}[BOB   ]${NC} Task submitted: $TASK_ID"
    echo ""

    # Poll for result
    RESULT=$(poll_task "$BOB_URL" "$TASK_ID" 60)
    echo ""

    # Extract and display the response content
    CONTENT=$(echo "$RESULT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    parts = d.get('result', {}).get('result', {}).get('parts', [])
    for part in parts:
        if part.get('type') == 'text':
            print(part.get('content', ''))
except:
    pass
" 2>/dev/null)

    if [ -n "$CONTENT" ]; then
        echo -e "${GREEN}[BOB   ]${NC} Response:"
        echo "─────────────────────────────────────────────────"
        echo "$CONTENT" | head -50
        echo "─────────────────────────────────────────────────"
    fi
else
    echo -e "${RED}[ERROR ]${NC} Failed to submit task"
    echo "$RESPONSE"
fi

echo ""
echo "  Watch logs/bob.log for peer collaboration details"
echo ""
