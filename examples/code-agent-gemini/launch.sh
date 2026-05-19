#!/bin/bash

# Gemini Code Agent Launcher
# Starts an Arkavo agent with Google Gemini 2.5 Pro/Flash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
AGENT_NAME="gemini-code-agent"
AGENT_PORT=8346
AGENT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_DIR="$AGENT_DIR/workspace"
LOGS_DIR="$AGENT_DIR/logs"
PID_FILE="$LOGS_DIR/${AGENT_NAME}.pid"
LOG_FILE="$LOGS_DIR/${AGENT_NAME}.log"

# Arkavo binary location
ARKAVO_BIN="${ARKAVO_BIN:-../../target/debug/arkavo}"
if [ ! -f "$ARKAVO_BIN" ]; then
    ARKAVO_BIN="../../target/release/arkavo"
fi

# Create necessary directories
mkdir -p "$WORKSPACE_DIR" "$LOGS_DIR"

# Functions
print_status() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

check_prerequisites() {
    print_status "Checking prerequisites..."

    # Check API credentials
    if [ -z "$GEMINI_API_KEY" ]; then
        print_error "GEMINI_API_KEY not set!"
        echo ""
        echo "Get your API key from: https://aistudio.google.com/app/apikey"
        echo "Then run:"
        echo "  export GEMINI_API_KEY='your-api-key'"
        exit 1
    fi
    print_status "Gemini API key found ✓"

    # Check Arkavo binary with gemini feature
    if [ ! -f "$ARKAVO_BIN" ]; then
        print_error "Arkavo binary not found at $ARKAVO_BIN"
        echo "Please build Arkavo with gemini feature:"
        echo "  cd ../.. && cargo build --features gemini"
        exit 1
    fi
    print_status "Arkavo binary found ✓"

    # Verify gemini feature is compiled in
    if ! "$ARKAVO_BIN" --help 2>&1 | grep -q "gemini" && [ "${SKIP_FEATURE_CHECK:-0}" != "1" ]; then
        print_warning "Gemini feature may not be enabled in binary"
        echo "Rebuild with: cargo build --features gemini"
    fi
}

check_optional_tools() {
    print_status "Checking optional MCP tools..."

    local missing_tools=()

    # Code search tools
    if ! command -v rg &> /dev/null; then
        missing_tools+=("ripgrep (brew install ripgrep)")
    else
        print_status "ripgrep ✓"
    fi

    if ! command -v comby &> /dev/null; then
        missing_tools+=("comby (brew install comby)")
    else
        print_status "comby ✓"
    fi

    # Security tools
    if ! command -v semgrep &> /dev/null; then
        missing_tools+=("semgrep (brew install semgrep)")
    else
        print_status "semgrep ✓"
    fi

    if ! command -v osv-scanner &> /dev/null; then
        missing_tools+=("osv-scanner (pip install osv-scanner)")
    else
        print_status "osv-scanner ✓"
    fi

    if ! command -v syft &> /dev/null; then
        missing_tools+=("syft (brew install syft)")
    else
        print_status "syft ✓"
    fi

    if [ ${#missing_tools[@]} -gt 0 ]; then
        print_warning "Optional tools not installed:"
        for tool in "${missing_tools[@]}"; do
            echo "  - $tool"
        done
        echo ""
        echo "Agent will work without these, but some MCP tools will be unavailable."
        echo "Install them for full functionality."
    fi
}

start_agent() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null 2>&1; then
            print_warning "Agent already running (PID: $PID)"
            return
        fi
    fi

    print_status "Starting $AGENT_NAME on port $AGENT_PORT..."

    # Set model based on flag
    if [ -z "$GEMINI_MODEL" ]; then
        GEMINI_MODEL="gemini-3.5-flash"
    fi
    print_status "Using model: $GEMINI_MODEL"

    # Start the agent
    cd "$AGENT_DIR"
    nohup "$ARKAVO_BIN" agent run \
        --port "$AGENT_PORT" \
        --config "AGENTS.md" \
        > "$LOG_FILE" 2>&1 &

    PID=$!
    echo $PID > "$PID_FILE"

    # Wait for agent to start
    sleep 2

    # Check if agent started successfully
    if ps -p "$PID" > /dev/null 2>&1; then
        print_status "Agent started successfully (PID: $PID)"
        print_status "Logs: tail -f $LOG_FILE"

        # Test health endpoint
        sleep 1
        if curl -s "http://localhost:$AGENT_PORT/.well-known/agent.json" > /dev/null 2>&1; then
            print_status "Health check passed ✓"
        else
            print_warning "Health check failed - agent may still be initializing"
        fi
    else
        print_error "Failed to start agent"
        if [ -f "$LOG_FILE" ]; then
            echo "Last 20 lines of log:"
            tail -20 "$LOG_FILE"
        fi
        exit 1
    fi
}

stop_agent() {
    if [ ! -f "$PID_FILE" ]; then
        print_warning "No PID file found"
        return
    fi

    PID=$(cat "$PID_FILE")
    if ps -p "$PID" > /dev/null 2>&1; then
        print_status "Stopping agent (PID: $PID)..."
        kill "$PID"
        sleep 2

        if ps -p "$PID" > /dev/null 2>&1; then
            print_warning "Agent didn't stop gracefully, forcing..."
            kill -9 "$PID"
        fi

        rm -f "$PID_FILE"
        print_status "Agent stopped"
    else
        print_warning "Agent not running"
        rm -f "$PID_FILE"
    fi
}

status_agent() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p "$PID" > /dev/null 2>&1; then
            print_status "Agent is running (PID: $PID)"

            # Check health
            if curl -s "http://localhost:$AGENT_PORT/.well-known/agent.json" > /dev/null 2>&1; then
                print_status "Health check: HEALTHY"
            else
                print_warning "Health check: UNREACHABLE"
            fi

            # Show recent logs
            if [ -f "$LOG_FILE" ]; then
                echo ""
                echo "Recent logs:"
                tail -5 "$LOG_FILE"
            fi
        else
            print_warning "Agent not running (stale PID file)"
            rm -f "$PID_FILE"
        fi
    else
        print_status "Agent is not running"
    fi
}

restart_agent() {
    stop_agent
    sleep 1
    start_agent
}

logs_agent() {
    if [ -f "$LOG_FILE" ]; then
        print_status "Following logs (Ctrl+C to exit)..."
        tail -f "$LOG_FILE"
    else
        print_error "No log file found"
    fi
}

test_connection() {
    print_status "Testing Gemini API connection..."

    # Test via chat command
    RESPONSE=$(cd ../.. && cargo run --features gemini -p arkavo -- \
        chat --model "${GEMINI_MODEL:-gemini-3.5-flash}" \
        --prompt "Respond with: API test successful" 2>&1)

    if [ $? -eq 0 ]; then
        print_status "Gemini API test successful"
        echo "$RESPONSE"
    else
        print_error "Gemini API test failed"
        echo "$RESPONSE"
        exit 1
    fi
}

show_budget() {
    print_status "Checking budget status..."

    RESPONSE=$(curl -s "http://localhost:$AGENT_PORT/v1/agent/budget" 2>&1)
    if [ $? -eq 0 ]; then
        echo "$RESPONSE" | jq '.' 2>/dev/null || echo "$RESPONSE"
    else
        print_error "Failed to get budget status"
    fi
}

# Main command handling
case "${1:-start}" in
    start)
        check_prerequisites
        if [ "${2}" = "--check-deps" ]; then
            check_optional_tools
            exit 0
        fi
        start_agent
        ;;
    stop)
        stop_agent
        ;;
    restart)
        restart_agent
        ;;
    status)
        status_agent
        ;;
    logs)
        logs_agent
        ;;
    test)
        test_connection
        ;;
    budget)
        show_budget
        ;;
    --flash)
        export GEMINI_MODEL="gemini-3.5-flash"
        check_prerequisites
        start_agent
        ;;
    --flash-legacy)
        export GEMINI_MODEL="gemini-flash-latest"
        check_prerequisites
        start_agent
        ;;
    --lite)
        export GEMINI_MODEL="gemini-flash-lite-latest"
        check_prerequisites
        start_agent
        ;;
    --pro)
        export GEMINI_MODEL="gemini-3-pro-preview"
        check_prerequisites
        start_agent
        ;;
    --check-deps)
        check_optional_tools
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status|logs|test|budget} [options]"
        echo ""
        echo "Commands:"
        echo "  start           - Start the Gemini Code agent (default: gemini-3.5-flash)"
        echo "  stop            - Stop the agent"
        echo "  restart         - Restart the agent"
        echo "  status          - Check agent status"
        echo "  logs            - Follow agent logs"
        echo "  test            - Test Gemini API connection"
        echo "  budget          - Show budget usage"
        echo ""
        echo "Model Options:"
        echo "  --pro           - Use Gemini 3 Pro Preview (best quality)"
        echo "  --flash         - Use Gemini 3.5 Flash (frontier reasoning, ~280 tok/s) [default]"
        echo "  --flash-legacy  - Use legacy gemini-flash-latest alias"
        echo "  --lite          - Use Gemini Flash-Lite (fastest, cheapest)"
        echo ""
        echo "Other Options:"
        echo "  --check-deps    - Check optional tool dependencies"
        echo ""
        echo "Examples:"
        echo "  $0 start --flash           # Start with Flash model"
        echo "  $0 start --check-deps      # Check dependencies before starting"
        exit 1
        ;;
esac
