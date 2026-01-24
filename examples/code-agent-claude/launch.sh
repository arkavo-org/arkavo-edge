#!/bin/bash

# Claude Code Agent Launcher
# Starts an Arkavo agent with Claude Code capability

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
AGENT_NAME="claude-code-agent"
AGENT_PORT=8345
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

    # Check Arkavo binary
    if [ ! -f "$ARKAVO_BIN" ]; then
        print_error "Arkavo binary not found at $ARKAVO_BIN"
        echo "Please build Arkavo first:"
        echo "  cd ../.. && cargo build"
        exit 1
    fi
    print_status "Arkavo binary found ✓"

    # Check authentication
    if [ -n "$ANTHROPIC_API_KEY" ]; then
        print_status "Using API key authentication ✓"
    else
        print_status "Using OAuth authentication (Claude Max/Pro)"
        print_status "If not authenticated, run: claude login"
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
    print_status "Testing Claude Code SDK connection..."
    
    # Simple test request
    RESPONSE=$(curl -s -X POST "http://localhost:$AGENT_PORT/v1/agent/test" \
        -H "Content-Type: application/json" \
        -d '{
            "tool": "claude_code_plan",
            "prompt": "Say hello and confirm you are working"
        }' 2>&1)
    
    if [ $? -eq 0 ]; then
        print_status "Connection test successful"
        echo "$RESPONSE" | jq '.' 2>/dev/null || echo "$RESPONSE"
    else
        print_error "Connection test failed"
        echo "$RESPONSE"
    fi
}

# Main command handling
case "${1:-start}" in
    start)
        check_prerequisites
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
    *)
        echo "Usage: $0 {start|stop|restart|status|logs|test}"
        echo ""
        echo "Commands:"
        echo "  start   - Start the Claude Code agent"
        echo "  stop    - Stop the agent"
        echo "  restart - Restart the agent"
        echo "  status  - Check agent status"
        echo "  logs    - Follow agent logs"
        echo "  test    - Test Claude Code SDK connection"
        echo ""
        echo "Authentication:"
        echo "  OAuth (default): claude login"
        echo "  API key: export ANTHROPIC_API_KEY='sk-ant-...'"
        exit 1
        ;;
esac