#!/bin/bash
# Family Travel Mesh - HRM Orchestration Demo
# Launches all HRM components in correct order

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
if [ ! -f "$BINARY" ]; then
    BINARY="${PROJECT_ROOT}/target/release/arkavo"
fi
LOG_DIR="$SCRIPT_DIR/logs"
PIDS_FILE="$SCRIPT_DIR/.pids"

mkdir -p "$LOG_DIR"

print_status() {
    local status=$1
    local message=$2
    case $status in
        "INFO") echo -e "${BLUE}[INFO]${NC} $message" ;;
        "SUCCESS") echo -e "${GREEN}[SUCCESS]${NC} $message" ;;
        "ERROR") echo -e "${RED}[ERROR]${NC} $message" ;;
        "MESH") echo -e "${CYAN}[MESH]${NC} $message" ;;
    esac
}

check_binary() {
    if [ ! -f "$BINARY" ]; then
        print_status "ERROR" "Binary not found at $BINARY"
        print_status "INFO" "Please run: cargo build"
        exit 1
    fi
}

check_port() {
    local port=$1
    if lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
        return 1
    fi
    return 0
}

stop_agents() {
    print_status "INFO" "Stopping existing agents..."
    if [ -f "$PIDS_FILE" ]; then
        while IFS=':' read -r pid name; do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
            fi
        done < "$PIDS_FILE"
        rm -f "$PIDS_FILE"
    fi
    pkill -f "arkavo agent" 2>/dev/null || true
}

KIT="$SCRIPT_DIR/family-travel-mesh.swarmkit.yaml"

start_agent() {
    local name=$1
    local role=$2
    local port=$3
    local log_file="$LOG_DIR/${name}.log"

    print_status "MESH" "Starting $name on port $port..."

    if ! check_port $port; then
        print_status "ERROR" "Port $port is in use"
        return 1
    fi

    nohup "$BINARY" agent -c "$KIT" -n "$role" -p "$port" > "$log_file" 2>&1 &
    local pid=$!
    echo "$pid:$name" >> "$PIDS_FILE"

    local max_attempts=15
    local attempt=0
    while [ $attempt -lt $max_attempts ]; do
        if curl -s "http://localhost:$port/.well-known/agent.json" > /dev/null 2>&1; then
            print_status "SUCCESS" "$name started (PID: $pid)"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done

    print_status "ERROR" "$name failed to start"
    return 1
}

verify_mesh() {
    print_status "INFO" "Verifying mesh connectivity..."
    local all_healthy=true

    local agents=("conductor:8401" "router:8402" "critic:8403" "memory:8404"
                  "vegas-guide:8410" "family-activities:8411" "budget-optimizer:8412")

    for agent in "${agents[@]}"; do
        IFS=':' read -r name port <<< "$agent"
        if curl -s "http://localhost:$port/.well-known/agent.json" > /dev/null 2>&1; then
            print_status "SUCCESS" "$name is healthy"
        else
            print_status "ERROR" "$name is not responding"
            all_healthy=false
        fi
    done

    if [ "$all_healthy" = true ]; then
        return 0
    else
        return 1
    fi
}

main() {
    echo ""
    echo "========================================"
    echo " ARKAVO HRM MESH - Family Travel Demo  "
    echo " Issue #236 Implementation             "
    echo "========================================"
    echo ""

    case "${1:-start}" in
        stop)
            stop_agents
            print_status "SUCCESS" "All agents stopped"
            exit 0
            ;;
        status)
            verify_mesh
            exit $?
            ;;
        restart)
            stop_agents
            sleep 2
            ;;
        help|--help|-h)
            echo "Usage: $0 [start|stop|restart|status|help]"
            exit 0
            ;;
    esac

    check_binary
    stop_agents
    > "$PIDS_FILE"

    print_status "INFO" "Starting HRM mesh components..."
    echo ""

    # Start in dependency order: Memory -> Critic -> Specialists -> Router -> Conductor
    # (name : role id in family-travel-mesh.swarmkit.yaml : port)
    start_agent "memory" "memory-service" 8404 || exit 1
    sleep 1

    start_agent "critic" "family-safety-critic" 8403 || exit 1
    sleep 1

    start_agent "vegas-guide" "vegas-guide" 8410 || exit 1
    start_agent "family-activities" "family-activities" 8411 || exit 1
    start_agent "budget-optimizer" "budget-optimizer" 8412 || exit 1
    sleep 1

    start_agent "router" "agent-router" 8402 || exit 1
    sleep 1

    start_agent "conductor" "family-travel-conductor" 8401 || exit 1

    echo ""
    print_status "INFO" "Waiting for mDNS discovery (5s)..."
    sleep 5

    echo ""
    if verify_mesh; then
        echo ""
        print_status "SUCCESS" "HRM mesh is ready!"
        echo ""
        echo "Agent Endpoints:"
        echo "  Conductor:         http://localhost:8401"
        echo "  Router:            http://localhost:8402"
        echo "  Critic:            http://localhost:8403"
        echo "  Memory:            http://localhost:8404"
        echo "  Vegas Guide:       http://localhost:8410"
        echo "  Family Activities: http://localhost:8411"
        echo "  Budget Optimizer:  http://localhost:8412"
        echo ""
        print_status "INFO" "Run './run_task.sh' to execute the demo"
        print_status "INFO" "Run './stop.sh' to stop all agents"
        echo ""
    else
        print_status "ERROR" "Failed to start all agents"
        stop_agents
        exit 1
    fi
}

main "$@"
