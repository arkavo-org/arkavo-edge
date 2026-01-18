#!/bin/bash
# Minecraft Survival Swarm - HRM Multi-Agent Demo
# Launches 5-agent swarm controlling a single Minecraft bot

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
        "SWARM") echo -e "${CYAN}[SWARM]${NC} $message" ;;
        "MINECRAFT") echo -e "${GREEN}[MINECRAFT]${NC} $message" ;;
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

start_agent() {
    local name=$1
    local dir=$2
    local port=$3
    local log_file="$LOG_DIR/${name}.log"

    print_status "SWARM" "Starting $name on port $port..."

    if ! check_port $port; then
        print_status "ERROR" "Port $port is in use"
        return 1
    fi

    cd "$dir"
    nohup env RUST_LOG=info "$BINARY" agent --config AGENTS.md > "$log_file" 2>&1 &
    local pid=$!
    echo "$pid:$name" >> "$PIDS_FILE"
    cd "$SCRIPT_DIR"

    local max_attempts=30
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

start_minecraft() {
    print_status "MINECRAFT" "Starting Minecraft server..."

    if ! docker info &> /dev/null; then
        print_status "ERROR" "Docker daemon is not running"
        exit 1
    fi

    cd "$SCRIPT_DIR"
    docker compose up -d

    print_status "INFO" "Waiting for server to be healthy..."
    for i in {1..90}; do
        if docker compose ps minecraft 2>/dev/null | grep -q "healthy"; then
            print_status "SUCCESS" "Minecraft server is healthy!"
            break
        fi
        if [ $i -eq 90 ]; then
            print_status "ERROR" "Server failed to become healthy"
            docker compose logs minecraft | tail -20
            exit 1
        fi
        echo -n "."
        sleep 2
    done

    print_status "INFO" "Waiting for MCP container..."
    for i in {1..30}; do
        if docker compose ps mcp 2>/dev/null | grep -q "Up"; then
            print_status "SUCCESS" "MCP container ready!"
            break
        fi
        if [ $i -eq 30 ]; then
            print_status "ERROR" "MCP container failed to start"
            exit 1
        fi
        sleep 1
    done
}

verify_swarm() {
    print_status "INFO" "Verifying swarm connectivity..."
    local all_healthy=true

    local agents=("commander:8401" "router:8402" "scout:8410" "builder:8411" "runner:8412")

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
    echo " MINECRAFT SURVIVAL SWARM              "
    echo " 5-Agent HRM Demo                      "
    echo "========================================"
    echo ""

    case "${1:-start}" in
        stop)
            stop_agents
            docker compose down 2>/dev/null || true
            print_status "SUCCESS" "All agents and server stopped"
            exit 0
            ;;
        status)
            verify_swarm
            exit $?
            ;;
        restart)
            stop_agents
            docker compose down 2>/dev/null || true
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

    # Start Minecraft server first
    start_minecraft
    echo ""

    print_status "INFO" "Starting HRM swarm agents..."
    echo ""

    # Start in dependency order: Specialists -> Router -> Commander
    start_agent "scout" "$SCRIPT_DIR/agents/specialists/scout" 8410 || exit 1
    start_agent "builder" "$SCRIPT_DIR/agents/specialists/builder" 8411 || exit 1
    start_agent "runner" "$SCRIPT_DIR/agents/specialists/runner" 8412 || exit 1
    sleep 1

    start_agent "router" "$SCRIPT_DIR/agents/router" 8402 || exit 1
    sleep 1

    start_agent "commander" "$SCRIPT_DIR/agents/commander" 8401 || exit 1

    echo ""
    print_status "INFO" "Waiting for mDNS discovery (5s)..."
    sleep 5

    echo ""
    if verify_swarm; then
        echo ""
        print_status "SUCCESS" "Minecraft Survival Swarm is ready!"
        echo ""
        echo "Agent Endpoints:"
        echo "  Commander: http://localhost:8401  (has MCP tools)"
        echo "  Router:    http://localhost:8402"
        echo "  Scout:     http://localhost:8410"
        echo "  Builder:   http://localhost:8411"
        echo "  Runner:    http://localhost:8412"
        echo ""
        echo "Minecraft: localhost:25565 (connect with client)"
        echo ""
        print_status "INFO" "Logs in: $LOG_DIR/"
        print_status "INFO" "Stop: ./launch_minecraft.sh stop"
        echo ""
    else
        print_status "ERROR" "Failed to start all agents"
        stop_agents
        exit 1
    fi
}

main "$@"
