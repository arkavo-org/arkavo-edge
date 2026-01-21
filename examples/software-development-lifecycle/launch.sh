#!/bin/bash
# Launch Multi-Agent Software Development System
# Starts all 11 specialized agents for collaborative development

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
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
    esac
}

check_prerequisites() {
    local missing=0

    # Check arkavo binary
    if [ ! -f "$BINARY" ]; then
        print_status "ERROR" "Arkavo binary not found. Run: cargo build"
        missing=1
    else
        print_status "SUCCESS" "Arkavo binary found"
    fi

    # Check npm/npx (optional - for external MCP servers)
    if command -v npx &> /dev/null; then
        print_status "SUCCESS" "npx available (for MCP servers)"

        # Check for optional MCP servers
        if npx --yes @modelcontextprotocol/server-filesystem --help &> /dev/null 2>&1; then
            print_status "SUCCESS" "MCP filesystem server available"
        else
            print_status "WARNING" "MCP filesystem server not installed (optional)"
            print_status "INFO" "Install with: npm install -g @modelcontextprotocol/server-filesystem"
        fi
    else
        print_status "WARNING" "npx not found - external MCP servers unavailable"
        print_status "INFO" "Agents will work with reduced capabilities"
    fi

    return $missing
}

stop_agents() {
    print_status "INFO" "Stopping all agents..."

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

start_agent() {
    local name=$1
    local dir=$2
    local port=$3

    if [ ! -d "$dir" ]; then
        print_status "WARNING" "Directory $dir not found, skipping $name"
        return 0
    fi

    if [ ! -f "$dir/AGENTS.md" ]; then
        print_status "WARNING" "No AGENTS.md in $dir, skipping $name"
        return 0
    fi

    print_status "INFO" "Starting $name (port $port)..."

    cd "$dir"
    nohup "$BINARY" agent run > "$LOG_DIR/${name}.log" 2>&1 &
    echo "$!" >> "$PID_FILE"

    cd "$SCRIPT_DIR"
}

start_all_agents() {
    > "$PID_FILE"

    # Core agents
    start_agent "orchestrator" "$SCRIPT_DIR/orchestrator" 8342
    start_agent "security" "$SCRIPT_DIR/security" 8343
    start_agent "code-review" "$SCRIPT_DIR/code-review" 8344
    start_agent "database" "$SCRIPT_DIR/database" 8345
    start_agent "testing" "$SCRIPT_DIR/testing" 8346
    start_agent "documentation" "$SCRIPT_DIR/documentation" 8347
    start_agent "performance" "$SCRIPT_DIR/performance" 8348
    start_agent "devops" "$SCRIPT_DIR/devops" 8349
    start_agent "frontend" "$SCRIPT_DIR/frontend" 8350
    start_agent "architecture" "$SCRIPT_DIR/architecture" 8351
    start_agent "data-science" "$SCRIPT_DIR/data-science" 8352
    start_agent "debug" "$SCRIPT_DIR/debug" 8353

    print_status "INFO" "Waiting for agents to initialize..."
    sleep 5
}

show_status() {
    echo ""
    echo "Agent Status:"
    echo "============="

    local agents=("orchestrator:8342" "security:8343" "code-review:8344"
                  "database:8345" "testing:8346" "documentation:8347"
                  "performance:8348" "devops:8349" "frontend:8350"
                  "architecture:8351" "data-science:8352" "debug:8353")

    for agent in "${agents[@]}"; do
        local name="${agent%:*}"
        local port="${agent#*:}"
        if curl -s "http://localhost:$port/.well-known/agent.json" > /dev/null 2>&1; then
            print_status "SUCCESS" "$name (port $port)"
        else
            print_status "WARNING" "$name (port $port) - not responding"
        fi
    done
}

main() {
    echo ""
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║     ARKAVO - Multi-Agent Software Development System          ║"
    echo "║                   11 Specialized Agents                       ║"
    echo "╚═══════════════════════════════════════════════════════════════╝"
    echo ""

    case "${1:-start}" in
        stop)
            stop_agents
            ;;
        restart)
            stop_agents
            sleep 2
            check_prerequisites || exit 1
            start_all_agents
            show_status
            ;;
        status)
            show_status
            ;;
        start)
            check_prerequisites || exit 1
            stop_agents
            start_all_agents
            show_status
            echo ""
            print_status "INFO" "Logs: $LOG_DIR/"
            print_status "INFO" "Stop: $0 stop"
            ;;
        *)
            echo "Usage: $0 [start|stop|restart|status]"
            exit 1
            ;;
    esac
}

main "$@"
