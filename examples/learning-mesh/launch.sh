#!/bin/bash
# Launch Learning Mesh - Quality-Aware Agent Routing
# Demonstrates lesson-informed prompting with Thompson Sampling

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
PID_FILE="$SCRIPT_DIR/.agent_pids"

mkdir -p "$LOG_DIR"

print_status() {
    local status=$1
    local message=$2
    case $status in
        "INFO") echo -e "${BLUE}[INFO]${NC} $message" ;;
        "SUCCESS") echo -e "${GREEN}[OK]${NC} $message" ;;
        "ERROR") echo -e "${RED}[ERROR]${NC} $message" ;;
        "WARNING") echo -e "${YELLOW}[WARN]${NC} $message" ;;
        "AGENT") echo -e "${CYAN}[AGENT]${NC} $message" ;;
    esac
}

check_prerequisites() {
    if [ ! -f "$BINARY" ]; then
        print_status "ERROR" "Arkavo binary not found. Run: cargo build"
        return 1
    fi
    print_status "SUCCESS" "Arkavo binary found"
}

stop_agents() {
    print_status "INFO" "Stopping learning mesh..."
    if [ -f "$PID_FILE" ]; then
        while IFS= read -r pid; do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
            fi
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi
    pkill -f "arkavo agent" 2>/dev/null || true
    print_status "SUCCESS" "All agents stopped"
}

KIT="$SCRIPT_DIR/learning-mesh.swarmkit.yaml"

start_agent() {
    local name=$1
    local role=$2
    local port=$3

    print_status "AGENT" "Starting $name (port $port)..."

    RUST_LOG=info nohup "$BINARY" agent -c "$KIT" -n "$role" -p "$port" > "$LOG_DIR/${name}.log" 2>&1 &
    echo "$!" >> "$PID_FILE"
}

start_mesh() {
    > "$PID_FILE"

    start_agent "orchestrator" "learning-orchestrator" 8410
    sleep 1
    start_agent "code-analyzer" "code-analyzer-agent" 8412
    start_agent "test-generator" "test-generator-agent" 8414
    start_agent "security-auditor" "security-auditor-agent" 8416
    start_agent "task-generator" "task-generator-agent" 8418

    print_status "INFO" "Waiting for agents to initialize and discover peers..."
    sleep 3
}

show_status() {
    echo ""
    echo "Learning Mesh Status:"
    echo "====================="

    local agents=("orchestrator:8410" "code-analyzer:8412" "test-generator:8414" "security-auditor:8416" "task-generator:8418")

    for agent in "${agents[@]}"; do
        local name="${agent%:*}"
        local port="${agent#*:}"
        if curl -s "http://localhost:$port/.well-known/agent.json" > /dev/null 2>&1; then
            print_status "SUCCESS" "$name (port $port)"
        else
            print_status "WARNING" "$name (port $port) - initializing..."
        fi
    done
}

main() {
    echo ""
    echo "=========================================================="
    echo "  ARKAVO - Learning Mesh"
    echo "  Quality-Aware Routing with Lesson-Informed Prompting"
    echo "=========================================================="
    echo ""

    case "${1:-start}" in
        stop)
            stop_agents
            ;;
        restart)
            stop_agents
            sleep 2
            check_prerequisites || exit 1
            start_mesh
            show_status
            ;;
        status)
            show_status
            ;;
        start)
            check_prerequisites || exit 1
            stop_agents
            start_mesh
            show_status
            echo ""
            print_status "INFO" "Logs: $LOG_DIR/"
            print_status "INFO" "Stop: $0 stop"
            print_status "INFO" "UI:   cargo run -p arkavo -- ui 7700"
            echo ""
            echo "Submit tasks via the AG-UI or curl:"
            echo "  curl -X POST http://localhost:8410/tasks -d @tasks.json"
            echo ""
            echo "Watch the learning loop:"
            echo "  tail -f $LOG_DIR/orchestrator.log | grep -E 'Lesson extracted|Injecting.*guidance|quality='"
            ;;
        *)
            echo "Usage: $0 [start|stop|restart|status]"
            exit 1
            ;;
    esac
}

main "$@"
