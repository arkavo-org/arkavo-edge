#!/bin/bash
# Secure Data Plane: TDF + KAS + Iroh P2P Demo

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"
PID_FILE="${SCRIPT_DIR}/logs/pids"
source "${SCRIPT_DIR}/../common/run_agent.sh"

echo -e "${CYAN}Secure Data Plane Demo${NC}"
echo -e "${CYAN}TDF + KAS + Iroh P2P${NC}"
echo "========================"
echo ""

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}Error: Arkavo binary not found${NC}"
    echo "Build with: cargo build --features kas,iroh"
    exit 1
fi

# Clean up previous run
rm -f "$PID_FILE"

echo -e "${YELLOW}Architecture:${NC}"
echo ""
echo "  ┌──────────────────┐    Iroh P2P    ┌──────────────────┐"
echo "  │   Data Sender    │ ─────────────> │  Data Receiver   │"
echo "  │                  │   (encrypted)  │                  │"
echo "  │ TDF Encrypt      │                │ KAS Rewrap       │"
echo "  │ Iroh Stage       │    tdf.share   │ Iroh Fetch       │"
echo "  │ kas.publicKey    │ <───────────── │ tdf.offers       │"
echo "  └──────────────────┘   (A2A JSON)   └──────────────────┘"
echo ""
echo -e "${GREEN}Starting agents...${NC}"
echo ""

# Start receiver first (it has the KAS for decryption)
start_agent "$BINARY" \
    "${SCRIPT_DIR}/receiver/AGENTS.md" \
    "${SCRIPT_DIR}/logs/receiver.log" \
    "$PID_FILE" \
    "data-receiver"

# Start sender
start_agent "$BINARY" \
    "${SCRIPT_DIR}/sender/AGENTS.md" \
    "${SCRIPT_DIR}/logs/sender.log" \
    "$PID_FILE" \
    "data-sender"

echo ""
print_agent_status "$PID_FILE"

echo ""
echo -e "${YELLOW}Test with:${NC}"
echo "  ./test-data-plane.sh"
echo ""
echo -e "${YELLOW}Stop with:${NC}"
echo "  ./stop.sh"
