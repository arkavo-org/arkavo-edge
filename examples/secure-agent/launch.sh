#!/bin/bash
# Secure Agent - Preflight Policy Demo

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Resolved as: $BINARY if set, then the source build, then arkavo on PATH.
source "${SCRIPT_DIR}/../common/resolve_binary.sh"
resolve_arkavo_binary || true

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

echo "Secure Agent - Preflight Policy Demo"
echo "====================================="
echo ""

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    explain_missing_arkavo_binary
    exit 1
fi

echo -e "${GREEN}Starting secure-agent with preflight policies...${NC}"
echo ""
echo "Configured policies:"
echo "  - block_pii: SSN, credit cards, emails"
echo "  - block_sql_injection: SQL keywords"
echo "  - block_shell_commands: Shell commands"
echo "  - block_long_input: >100KB inputs"
echo ""

cd "$SCRIPT_DIR"
"$BINARY" agent run
