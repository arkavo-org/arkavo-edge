#!/bin/bash
# RimWorld Survival Swarm - HRM Multi-Agent Demo
# Launches 4-agent swarm: 1 orchestrator + 3 specialists

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
RIMWORLD_SOCKET="/tmp/gamerl-rimworld.sock"

mkdir -p "$LOG_DIR"

print_status() {
    local status=$1
    local message=$2
    case $status in
        "INFO") echo -e "${BLUE}[INFO]${NC} $message" ;;
        "SUCCESS") echo -e "${GREEN}[SUCCESS]${NC} $message" ;;
        "ERROR") echo -e "${RED}[ERROR]${NC} $message" ;;
        "WARN") echo -e "${YELLOW}[WARN]${NC} $message" ;;
        "SWARM") echo -e "${CYAN}[SWARM]${NC} $message" ;;
        "RIMWORLD") echo -e "${GREEN}[RIMWORLD]${NC} $message" ;;
    esac
}

check_binary() {
    if [ ! -f "$BINARY" ]; then
        print_status "ERROR" "Binary not found at $BINARY"
        print_status "INFO" "Please run: cargo build -p arkavo"
        exit 1
    fi
}

check_socket() {
    if [ ! -S "$RIMWORLD_SOCKET" ]; then
        print_status "WARN" "RimWorld socket not found at $RIMWORLD_SOCKET"
        print_status "INFO" "Please start RimWorld with Arkavo Game-RL mod enabled first"
        print_status "INFO" "The mod creates the socket when the game loads"
        return 1
    fi
    return 0
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

KIT="$SCRIPT_DIR/rimworld-swarm.swarmkit.yaml"

start_agent() {
    local name=$1
    local role=$2
    local port=$3
    local log_file="$LOG_DIR/${name}.log"

    print_status "SWARM" "Starting $name on port $port..."

    if ! check_port $port; then
        print_status "ERROR" "Port $port is in use"
        return 1
    fi

    # Enable debug logging for chat/swarm interactions
    local log_level="${RUST_LOG:-arkavo_protocol=debug,arkavo_agui=debug,arkavo_router=info,arkavo_server=info,arkavo_llm=info}"
    nohup env ARKAVO_DEBUG=1 ARKAVO_DEBUG_PEG=1 RUST_LOG="$log_level" ${GEMINI_API_KEY:+GEMINI_API_KEY="$GEMINI_API_KEY"} ${ANTHROPIC_API_KEY:+ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY"} "$BINARY" agent -c "$KIT" -n "$role" -p "$port" > "$log_file" 2>&1 &
    local pid=$!
    echo "$pid:$name" >> "$PIDS_FILE"

    local max_attempts=30
    local attempt=0
    while [ $attempt -lt $max_attempts ]; do
        if curl -s "http://localhost:$port/health" > /dev/null 2>&1; then
            print_status "SUCCESS" "$name started (PID: $pid)"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done

    print_status "ERROR" "$name failed to start"
    return 1
}

verify_swarm() {
    print_status "INFO" "Verifying swarm connectivity..."
    local all_healthy=true

    local agents=("commander:8401" "survival:8410")
    # local agents=("commander:8401" "survival:8410" "industry:8411" "defense:8412" "historian:8413")

    for agent in "${agents[@]}"; do
        IFS=':' read -r name port <<< "$agent"
        if curl -s "http://localhost:$port/health" > /dev/null 2>&1; then
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
    echo " RIMWORLD SURVIVAL SWARM               "
    echo " 4-Agent HRM Demo                      "
    echo "========================================"
    echo ""

    case "${1:-start}" in
        stop)
            stop_agents
            print_status "SUCCESS" "All agents stopped"
            exit 0
            ;;
        status)
            verify_swarm
            exit $?
            ;;
        restart)
            stop_agents
            sleep 2
            ;;
        help|--help|-h)
            echo "Usage: $0 [start|stop|restart|status|help]"
            echo ""
            echo "Prerequisites:"
            echo "  1. Build Arkavo: cargo build -p arkavo"
            echo "  2. Build harmony-bridge: cargo build -p harmony-bridge (in game-rl repo)"
            echo "  3. Start RimWorld with GameRL mod enabled"
            echo "  4. Run this script to start agents"
            echo ""
            echo "Environment:"
            echo "  RUST_LOG  Set log level (default: arkavo_protocol=debug,arkavo_agui=debug,arkavo_router=info)"
            echo ""
            echo "Examples:"
            echo "  $0                          # Start with debug logging"
            echo "  RUST_LOG=info $0            # Start with info logging only"
            echo "  RUST_LOG=debug $0           # Start with full debug logging"
            exit 0
            ;;
    esac

    check_binary

    # Check for RimWorld socket (warn but don't fail)
    if ! check_socket; then
        print_status "WARN" "Continuing without RimWorld connection..."
        print_status "INFO" "Commander will fail to connect to MCP server"
        echo ""
    else
        print_status "RIMWORLD" "Socket found at $RIMWORLD_SOCKET"
    fi

    stop_agents
    > "$PIDS_FILE"

    print_status "INFO" "Starting HRM swarm agents..."
    echo ""

    # Start in dependency order: Specialists -> Commander (orchestrator)
    start_agent "survival" "survival" 8410 || exit 1
    #start_agent "industry" "industry" 8411 || exit 1
    #start_agent "defense" "defense" 8412 || exit 1
    #start_agent "historian" "historian" 8413 || exit 1
    sleep 1

    start_agent "commander" "commander" 8401 || exit 1

    echo ""
    print_status "INFO" "Waiting for mDNS discovery (5s)..."
    sleep 5

    echo ""
    if verify_swarm; then
        echo ""
        print_status "SUCCESS" "RimWorld Survival Swarm is ready!"
        echo ""
        echo "Agent Endpoints:"
        echo "  Commander: http://localhost:8401  (orchestrator, has MCP tools)"
        echo "  Survival:  http://localhost:8410"
        echo "  Industry:  http://localhost:8411"
        echo "  Defense:   http://localhost:8412"
        echo "  Historian: http://localhost:8413  (lesson synthesis)"
        echo ""
        echo "RimWorld: Connect via socket at $RIMWORLD_SOCKET"
        echo ""
        print_status "INFO" "Logs in: $LOG_DIR/"
        print_status "INFO" "Stop: ./launch_rimworld.sh stop"
        echo ""
    else
        print_status "ERROR" "Failed to start all agents"
        stop_agents
        exit 1
    fi
}

main "$@"
