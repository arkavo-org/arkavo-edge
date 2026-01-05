#!/bin/bash
# Run RLM security analysis on the synthetic codebase
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"
REPO_DIR="$SCRIPT_DIR/synthetic_repo"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

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

# Run via arkavo task (uses conductor with RLM)
echo "Submitting analysis task to Arkavo..."
echo ""

# Use ARKAVO_DEBUG to see RLM activation
ARKAVO_DEBUG=1 "$BINARY" chat --prompt "$TASK" 2>&1 | while IFS= read -r line; do
    if [[ "$line" == *"[RLM]"* ]]; then
        echo -e "${GREEN}$line${NC}"
    elif [[ "$line" == *"CRITICAL"* ]]; then
        echo -e "${RED}$line${NC}"
    elif [[ "$line" == *"HIGH"* ]]; then
        echo -e "${YELLOW}$line${NC}"
    elif [[ "$line" == *"MEDIUM"* ]]; then
        echo -e "${BLUE}$line${NC}"
    elif [[ "$line" == *"context_"* ]]; then
        echo -e "${CYAN}[TOOL] $line${NC}"
    else
        echo "$line"
    fi
done

echo ""
echo -e "${GREEN}━━━━━━ ANALYSIS COMPLETE ━━━━━━${NC}"
