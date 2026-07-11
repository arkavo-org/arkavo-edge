#!/bin/bash
# Manage the agent mesh with automatic port discovery

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${BINARY:-$SCRIPT_DIR/../target/debug/arkavo}"
PID_FILE="$SCRIPT_DIR/.mesh_pids"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

start_mesh() {
    local count="${1:-8}"
    mkdir -p "$SCRIPT_DIR/logs"
    > "$PID_FILE"

    if [ ! -f "$BINARY" ]; then
        echo -e "${RED}Binary not found at $BINARY${NC}"
        echo "Please run: cargo build"
        exit 1
    fi

    echo "Starting $count agents (automatic port assignment via mDNS)..."
    echo ""

    for i in $(seq 0 $((count - 1))); do
        local agent_dir="$SCRIPT_DIR/mesh/agent-$i"

        # Create agent directory if needed
        if [ ! -d "$agent_dir" ]; then
            mkdir -p "$agent_dir"
        fi

        # Clean persisted .arkavo config so a fresh kit (or the zero-config
        # default) is used on this start.
        rm -rf "$agent_dir/.arkavo" 2>/dev/null

        echo -n "  agent-$i... "

        cd "$agent_dir"
        if [ "$i" -eq 0 ]; then
            # agent-0 is the only mesh member with a purpose-built identity.
            nohup "$BINARY" agent -c "$SCRIPT_DIR/mesh/orchestrator.swarmkit.yaml" > "$SCRIPT_DIR/logs/agent-$i.log" 2>&1 &
        else
            nohup "$BINARY" agent run > "$SCRIPT_DIR/logs/agent-$i.log" 2>&1 &
        fi
        local pid=$!
        echo $pid >> "$PID_FILE"

        # Brief wait for startup
        sleep 1

        if kill -0 "$pid" 2>/dev/null; then
            echo -e "${GREEN}STARTED (pid $pid)${NC}"
        else
            echo -e "${RED}FAILED${NC}"
        fi
    done

    echo ""
    echo -e "${GREEN}Mesh started. Agents will register via mDNS.${NC}"
    echo "Use 'arkavo ui' to see discovered agents."
}

stop_mesh() {
    echo "Stopping agent mesh..."

    if [ -f "$PID_FILE" ]; then
        while read -r pid; do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
            fi
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi

    # Also kill any remaining arkavo agent processes
    pkill -f "arkavo agent" 2>/dev/null || true

    echo -e "${GREEN}Mesh stopped${NC}"
}

status_mesh() {
    echo "Agent Mesh Status"
    echo "================="
    echo ""
    echo "Checking PIDs from last start:"

    if [ -f "$PID_FILE" ]; then
        local i=0
        while read -r pid; do
            if kill -0 "$pid" 2>/dev/null; then
                echo -e "  agent-$i (pid $pid): ${GREEN}RUNNING${NC}"
            else
                echo -e "  agent-$i (pid $pid): ${RED}STOPPED${NC}"
            fi
            i=$((i + 1))
        done < "$PID_FILE"
    else
        echo "  No PID file found. Mesh may not be running."
    fi

    echo ""
    echo "For live agent discovery, run: arkavo ui"
}

case "$1" in
    start)
        start_mesh "$2"
        ;;
    stop)
        stop_mesh
        ;;
    status)
        status_mesh
        ;;
    restart)
        stop_mesh
        sleep 2
        start_mesh "$2"
        ;;
    *)
        echo "Usage: $0 {start [count]|stop|status|restart [count]}"
        echo ""
        echo "Commands:"
        echo "  start [n]  - Start n agents (default 8) with automatic ports"
        echo "  stop       - Stop all mesh agents"
        echo "  status     - Show agent PIDs"
        echo "  restart    - Stop and start agents"
        exit 1
        ;;
esac
