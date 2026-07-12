#!/bin/bash
# Launch Self-Improvement Agent Swarm
# Uses GLM-4.7-Flash for agent-driven development

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

    # Check for GLM-4.7-Flash model. Not in model_map.rs's local-edge-model
    # vocabulary, so the kit omits agent_provisioning.model and the router
    # falls back to its default local model unless GLM is already loaded —
    # see README.md.
    local hf_cache="${HF_HOME:-$HOME/.cache/huggingface}/hub"
    if [ -d "$hf_cache/models--unsloth--GLM-4.7-Flash-GGUF" ]; then
        print_status "SUCCESS" "GLM-4.7-Flash model available (unsloth)"
    else
        print_status "WARNING" "GLM-4.7-Flash not found. Run: arkavo model download glm"
    fi
}

stop_agents() {
    print_status "INFO" "Stopping self-improvement swarm..."
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

KIT="$SCRIPT_DIR/self-improvement-swarm.swarmkit.yaml"

start_agent() {
    local name=$1
    local role=$2
    local port=$3

    print_status "AGENT" "Starting $name (port $port) with GLM-4.7-Flash..."

    nohup "$BINARY" agent -c "$KIT" -n "$role" -p "$port" > "$LOG_DIR/${name}.log" 2>&1 &
    echo "$!" >> "$PID_FILE"
}

start_swarm() {
    > "$PID_FILE"

    # Start specialized agents
    start_agent "orchestrator" "self-improvement-orchestrator" 8400
    start_agent "code-analyzer" "code-analyzer-agent" 8401
    start_agent "refactorer" "refactorer-agent" 8402
    start_agent "test-generator" "test-generator-agent" 8403
    start_agent "performance-optimizer" "performance-optimizer-agent" 8404
    start_agent "clippy-fixer" "clippy-fixer-agent" 8405

    print_status "INFO" "Waiting for agents to initialize..."
    sleep 3
}

show_status() {
    echo ""
    echo "Self-Improvement Swarm Status:"
    echo "=============================="

    local agents=("orchestrator:8400" "code-analyzer:8401" "refactorer:8402"
                  "test-generator:8403" "performance-optimizer:8404" "clippy-fixer:8405")

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
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║   ARKAVO - Self-Improvement Agent Swarm                       ║"
    echo "║   Model: GLM-4.7-Flash (30B MoE)                              ║"
    echo "║   Agent-Driven Development                                    ║"
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
            start_swarm
            show_status
            ;;
        status)
            show_status
            ;;
        start)
            check_prerequisites || exit 1
            stop_agents
            start_swarm
            show_status
            echo ""
            print_status "INFO" "Logs: $LOG_DIR/"
            print_status "INFO" "Stop: $0 stop"
            print_status "INFO" "Interact: curl http://localhost:8400"
            ;;
        *)
            echo "Usage: $0 [start|stop|restart|status]"
            exit 1
            ;;
    esac
}

main "$@"
