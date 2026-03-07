#!/bin/bash
# Launch Vision Mesh - Image Analysis with Qwen3.5-27B Vision
# Requires: Qwen3.5-27B GGUF + mmproj file in HuggingFace cache

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

    local hf_cache="${HF_HOME:-$HOME/.cache/huggingface}/hub"
    local model_dir="$hf_cache/models--unsloth--Qwen3.5-27B-GGUF"

    if [ -d "$model_dir" ]; then
        print_status "SUCCESS" "Qwen3.5-27B model available"
    else
        print_status "ERROR" "Qwen3.5-27B not found."
        echo "  Download: hf download unsloth/Qwen3.5-27B-GGUF Qwen3.5-27B-UD-Q6_K_XL.gguf"
        return 1
    fi

    # Check for mmproj (vision projector)
    if find "$model_dir" -name "mmproj*.gguf" -print -quit 2>/dev/null | grep -q .; then
        print_status "SUCCESS" "Vision projector (mmproj) available"
    else
        print_status "ERROR" "Vision projector not found."
        echo "  Download: hf download unsloth/Qwen3.5-27B-GGUF mmproj-Qwen2.5-VL-7B-f16.gguf"
        return 1
    fi
}

stop_agents() {
    print_status "INFO" "Stopping vision mesh..."
    if [ -f "$PID_FILE" ]; then
        while IFS= read -r pid; do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
            fi
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi
    print_status "SUCCESS" "All agents stopped"
}

start_agent() {
    local name=$1
    local dir=$2
    local port=$3

    if [ ! -f "$dir/AGENTS.md" ]; then
        print_status "WARNING" "No AGENTS.md in $dir, skipping $name"
        return 0
    fi

    print_status "AGENT" "Starting $name (port $port)..."

    cd "$dir"
    RUST_LOG=info nohup "$BINARY" agent run > "$LOG_DIR/${name}.log" 2>&1 &
    echo "$!" >> "$PID_FILE"
    cd "$SCRIPT_DIR"
}

start_mesh() {
    > "$PID_FILE"

    start_agent "orchestrator" "$SCRIPT_DIR/orchestrator" 8418
    sleep 1
    start_agent "vision-analyst" "$SCRIPT_DIR/vision-analyst" 8420

    print_status "INFO" "Waiting for agents to initialize and discover peers..."
    sleep 3
}

show_status() {
    echo ""
    echo "Vision Mesh Status:"
    echo "==================="

    local agents=("orchestrator:8418" "vision-analyst:8420")

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
    echo "  ARKAVO - Vision Mesh"
    echo "  Image Analysis with Qwen3.5-27B Vision"
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
            echo ""
            echo "Test vision via A2A (JSON-RPC):"
            echo "  # Text-only task:"
            echo "  curl -s -X POST http://localhost:8418 \\"
            echo "    -H 'Content-Type: application/json' \\"
            echo "    -d '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"message/send\",\"params\":{\"request\":{\"message\":{\"parts\":[{\"type\":\"text\",\"content\":\"Describe the UI layout\"}]}}}}'"
            echo ""
            echo "  # With image (base64):"
            echo "  IMG=\$(base64 -i image.png | tr -d '\\n')"
            echo "  cat > /tmp/req.json << EOF"
            echo "  {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"message/send\",\"params\":{\"request\":{\"message\":{\"parts\":[{\"type\":\"text\",\"content\":\"Describe this image\"},{\"type\":\"file\",\"name\":\"image.png\",\"mime_type\":\"image/png\",\"data\":\"\${IMG}\",\"is_url\":false}]}}}}"
            echo "  EOF"
            echo "  curl -s -X POST http://localhost:8418 -H 'Content-Type: application/json' -d @/tmp/req.json"
            ;;
        *)
            echo "Usage: $0 [start|stop|restart|status]"
            exit 1
            ;;
    esac
}

main "$@"
