#!/bin/bash
# KAS A2A Capability Demo

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${CYAN}KAS as A2A Capability Demo${NC}"
echo "==========================="
echo ""

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}Error: Arkavo binary not found${NC}"
    echo "Please build with KAS feature: cargo build --features kas"
    exit 1
fi

echo -e "${GREEN}Starting KAS-enabled agent...${NC}"
echo ""
echo "Available JSON-RPC methods:"
echo "  - kas.publicKey  : Get KAS public key for TDF encryption"
echo "  - kas.rewrap     : Rewrap TDF keys with delegation verification"
echo ""
echo -e "${YELLOW}Endpoints:${NC}"
echo "  JSON-RPC : http://localhost:8080"
echo "  Agent Card: http://localhost:8081/.well-known/agent.json"
echo ""
echo -e "${YELLOW}Test with:${NC}"
echo '  curl -X POST http://localhost:8080 \'
echo '    -H "Content-Type: application/json" \'
echo '    -d '"'"'{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{}}'"'"
echo ""

cd "$SCRIPT_DIR"
"$BINARY" agent run
