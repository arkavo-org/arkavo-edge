#!/bin/bash
# Run RLM security analysis on the synthetic codebase
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"
REPO_DIR="$SCRIPT_DIR/synthetic_repo"
AGENT_PID=""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Cleanup function to stop agent on exit
cleanup() {
    if [ -n "$AGENT_PID" ]; then
        echo -e "\n${YELLOW}Stopping agent (PID: $AGENT_PID)...${NC}"
        kill "$AGENT_PID" 2>/dev/null || true
        wait "$AGENT_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

echo -e "${CYAN}━━━━━━ RLM Large Context Demo ━━━━━━${NC}"
echo ""

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}ERROR: Build arkavo first: cargo build${NC}"
    exit 1
fi

if [ ! -d "$REPO_DIR" ]; then
    echo -e "${YELLOW}Generating synthetic codebase...${NC}"
    ./generate_codebase.sh
fi

# Collect all source code
echo -e "${BLUE}━━━━━━ COLLECTING CODEBASE ━━━━━━${NC}"
echo ""

CODEBASE=""
for file in $(find "$REPO_DIR" -name "*.rs" | sort); do
    relative_path="${file#$REPO_DIR/}"
    CODEBASE+="
=== $relative_path ===
$(cat "$file")
"
done

# Calculate size
TOTAL_CHARS=${#CODEBASE}
APPROX_TOKENS=$((TOTAL_CHARS / 4))

echo "Codebase collected:"
echo "  Files: $(find "$REPO_DIR" -name "*.rs" | wc -l | tr -d ' ')"
echo "  Characters: $TOTAL_CHARS"
echo "  Approx tokens: ~$APPROX_TOKENS"
echo ""

# Check if RLM will activate
if [ $APPROX_TOKENS -gt 5700 ]; then
    echo -e "${GREEN}[RLM] Context exceeds 70% of 8K window - RLM will activate!${NC}"
else
    echo -e "${YELLOW}[RLM] Context within limits - RLM may not activate${NC}"
fi

echo ""
echo -e "${BLUE}━━━━━━ RUNNING SECURITY ANALYSIS ━━━━━━${NC}"
echo ""

# Create the analysis task
TASK="Analyze this codebase for security vulnerabilities. Focus on:
1. Password hashing issues
2. SQL injection vulnerabilities
3. Missing rate limiting
4. Any hardcoded secrets

Provide a security report with severity levels (CRITICAL, HIGH, MEDIUM, LOW).

CODEBASE:
$CODEBASE"

# Start Arkavo agent in background (provides RLM conductor)
echo -e "${BLUE}━━━━━━ STARTING AGENT ━━━━━━${NC}"
echo ""
echo "Starting Arkavo agent for RLM processing..."

# Start agent in background with debug output
# Use ARKAVO_OFFLINE=1 to force local models (better fence format support)
ARKAVO_DEBUG=1 ARKAVO_OFFLINE=1 "$BINARY" agent run > /tmp/arkavo-agent.log 2>&1 &
AGENT_PID=$!
echo "Agent started (PID: $AGENT_PID)"

# Wait for agent to be ready (mDNS advertisement takes a moment)
echo "Waiting for agent to advertise..."
sleep 3

# Verify agent is still running
if ! kill -0 "$AGENT_PID" 2>/dev/null; then
    echo -e "${RED}ERROR: Agent failed to start. Check /tmp/arkavo-agent.log${NC}"
    cat /tmp/arkavo-agent.log
    exit 1
fi

echo -e "${GREEN}Agent ready!${NC}"
echo ""

# Submit analysis via arkavo task (uses conductor with full RLM support)
echo -e "${BLUE}━━━━━━ SUBMITTING TASK ━━━━━━${NC}"
echo ""
echo "Submitting analysis task via mesh..."
echo ""

# Run task command - it will discover the agent and use RLM
ARKAVO_DEBUG=1 "$BINARY" task "$TASK" 2>&1 | while IFS= read -r line; do
    if [[ "$line" == *"RLM"* ]]; then
        echo -e "${GREEN}$line${NC}"
    elif [[ "$line" == *"decompos"* ]]; then
        echo -e "${GREEN}$line${NC}"
    elif [[ "$line" == *"CRITICAL"* ]]; then
        echo -e "${RED}$line${NC}"
    elif [[ "$line" == *"HIGH"* ]]; then
        echo -e "${YELLOW}$line${NC}"
    elif [[ "$line" == *"MEDIUM"* ]]; then
        echo -e "${BLUE}$line${NC}"
    elif [[ "$line" == *"context_"* ]]; then
        echo -e "${CYAN}[TOOL] $line${NC}"
    elif [[ "$line" == *"chunk"* ]]; then
        echo -e "${CYAN}$line${NC}"
    else
        echo "$line"
    fi
done

echo ""
echo -e "${GREEN}━━━━━━ ANALYSIS COMPLETE ━━━━━━${NC}"
