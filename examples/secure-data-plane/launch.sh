#!/bin/bash
# Secure Data Plane: TDF + KAS + Iroh P2P Demo

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"
PID_FILE="${SCRIPT_DIR}/logs/pids"
KIT="${SCRIPT_DIR}/secure-data-plane.swarmkit.yaml"
# Only sourced for colors and print_agent_status; start_agent's --config
# convention doesn't fit the -c/-n/-p kit surface, so start_kit_role below
# builds the invocation directly instead.
source "${SCRIPT_DIR}/../common/run_agent.sh"

# Start one role of the multi-role secure-data-plane kit on a fixed port
# (the ports below are the ones this demo's README and test script assume).
start_kit_role() {
    local binary="$1" kit="$2" role="$3" port="$4" log_file="$5" pid_file="$6" name="$7"

    mkdir -p "$(dirname "$log_file")"
    echo -e "${BLUE}Starting $name...${NC}"

    nohup "$binary" agent -c "$kit" -n "$role" -p "$port" >"$log_file" 2>&1 &
    local pid=$!
    echo "$pid $name" >>"$pid_file"

    sleep 0.5
    if ! ps -p "$pid" >/dev/null 2>&1; then
        echo -e "${RED}Error: $name failed to start${NC}"
        echo "Last log output:"
        tail -10 "$log_file" 2>/dev/null || true
        return 1
    fi

    echo -e "${GREEN}✓${NC} $name started (PID $pid)"
}

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
start_kit_role "$BINARY" "$KIT" "data-receiver" "8082" \
    "${SCRIPT_DIR}/logs/receiver.log" \
    "$PID_FILE" \
    "data-receiver"

# Start sender
start_kit_role "$BINARY" "$KIT" "data-sender" "8080" \
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
