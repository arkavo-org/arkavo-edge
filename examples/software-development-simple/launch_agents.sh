#!/bin/bash

# Agent Collaboration Demo - Launch Script
# This script starts all three agents for the collaboration demo

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
if [ ! -f "$BINARY" ]; then
    BINARY="${PROJECT_ROOT}/target/release/arkavo"
fi
LOG_DIR="$SCRIPT_DIR/logs"
PID_FILE="$SCRIPT_DIR/.agent_pids"

# Create log directory
mkdir -p "$LOG_DIR"

# Function to print colored output
print_status() {
    local status=$1
    local message=$2
    case $status in
        "INFO")
            echo -e "${BLUE}[INFO]${NC} $message"
            ;;
        "SUCCESS")
            echo -e "${GREEN}[SUCCESS]${NC} $message"
            ;;
        "ERROR")
            echo -e "${RED}[ERROR]${NC} $message"
            ;;
        "WARNING")
            echo -e "${YELLOW}[WARNING]${NC} $message"
            ;;
    esac
}

# Function to check if binary exists
check_binary() {
    if [ ! -f "$BINARY" ]; then
        print_status "ERROR" "Binary not found at $BINARY"
        print_status "INFO" "Please run: cargo build"
        exit 1
    fi
    print_status "SUCCESS" "Binary found at $BINARY"
}

# Function to check if port is available
check_port() {
    local port=$1
    if lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
        print_status "ERROR" "Port $port is already in use"
        return 1
    fi
    return 0
}

# Function to stop all agents
stop_agents() {
    print_status "INFO" "Stopping all agents..."
    
    if [ -f "$PID_FILE" ]; then
        while IFS= read -r pid; do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
                print_status "INFO" "Stopped process $pid"
            fi
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi
    
    # Also kill any arkavo agent processes
    pkill -f "arkavo agent" 2>/dev/null || true
    
    print_status "SUCCESS" "All agents stopped"
}

# Function to start an agent
start_agent() {
    local name=$1
    local dir=$2
    local port=$3
    local log_file="$LOG_DIR/${name}.log"
    
    print_status "INFO" "Starting $name on port $port..."
    
    # Check if port is available
    if ! check_port $port; then
        print_status "ERROR" "Cannot start $name - port $port is in use"
        return 1
    fi
    
    # Change to agent directory and start agent
    cd "$dir"
    
    # Start agent in background and capture PID
    nohup "$BINARY" agent run > "$log_file" 2>&1 &
    local pid=$!
    
    # Save PID
    echo "$pid" >> "$PID_FILE"
    
    # Wait for agent to start
    local max_attempts=30
    local attempt=0
    
    while [ $attempt -lt $max_attempts ]; do
        if curl -s "http://localhost:$port/health" > /dev/null 2>&1; then
            print_status "SUCCESS" "$name started successfully (PID: $pid)"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    
    print_status "ERROR" "$name failed to start within 30 seconds"
    return 1
}

# Function to verify all agents are running
verify_agents() {
    print_status "INFO" "Verifying agent connectivity..."
    
    local all_healthy=true
    
    # Check Project Manager
    if curl -s "http://localhost:8342/health" > /dev/null 2>&1; then
        print_status "SUCCESS" "Project Manager is healthy"
    else
        print_status "ERROR" "Project Manager is not responding"
        all_healthy=false
    fi
    
    # Check Coding Agent
    if curl -s "http://localhost:8343/health" > /dev/null 2>&1; then
        print_status "SUCCESS" "Coding Agent is healthy"
    else
        print_status "ERROR" "Coding Agent is not responding"
        all_healthy=false
    fi
    
    # Check Testing Agent
    if curl -s "http://localhost:8344/health" > /dev/null 2>&1; then
        print_status "SUCCESS" "Testing Agent is healthy"
    else
        print_status "ERROR" "Testing Agent is not responding"
        all_healthy=false
    fi
    
    if [ "$all_healthy" = true ]; then
        print_status "SUCCESS" "All agents are healthy and ready"
        return 0
    else
        print_status "ERROR" "Some agents are not healthy"
        return 1
    fi
}

# Function to show agent logs
show_logs() {
    print_status "INFO" "Agent logs are available at:"
    echo "  - Project Manager: $LOG_DIR/project-manager.log"
    echo "  - Coding Agent: $LOG_DIR/coding-agent.log"
    echo "  - Testing Agent: $LOG_DIR/testing-agent.log"
    echo ""
    print_status "INFO" "To monitor logs in real-time:"
    echo "  tail -f $LOG_DIR/*.log"
}

# Main execution
main() {
    echo "========================================="
    echo "Agent Collaboration Demo - Launch Script"
    echo "========================================="
    echo ""
    
    # Parse command line arguments
    case "${1:-}" in
        stop)
            stop_agents
            exit 0
            ;;
        restart)
            stop_agents
            sleep 2
            ;;
        status)
            verify_agents
            exit $?
            ;;
        logs)
            show_logs
            exit 0
            ;;
        help|--help|-h)
            echo "Usage: $0 [start|stop|restart|status|logs|help]"
            echo ""
            echo "Commands:"
            echo "  start    - Start all agents (default)"
            echo "  stop     - Stop all agents"
            echo "  restart  - Restart all agents"
            echo "  status   - Check agent health status"
            echo "  logs     - Show log file locations"
            echo "  help     - Show this help message"
            exit 0
            ;;
    esac
    
    # Check prerequisites
    check_binary
    
    # Stop any existing agents
    stop_agents
    
    # Clear PID file
    > "$PID_FILE"
    
    print_status "INFO" "Starting agent collaboration system..."
    echo ""
    
    # Start agents in order
    start_agent "project-manager" "$SCRIPT_DIR/project-manager" 8342
    sleep 2
    
    start_agent "coding-agent" "$SCRIPT_DIR/coding-agent" 8343
    sleep 2
    
    start_agent "testing-agent" "$SCRIPT_DIR/testing-agent" 8344
    sleep 2
    
    echo ""
    
    # Verify all agents are running
    if verify_agents; then
        echo ""
        print_status "SUCCESS" "Agent collaboration system is ready!"
        echo ""
        echo "Agent Endpoints:"
        echo "  - Project Manager: http://localhost:8342"
        echo "  - Coding Agent:    http://localhost:8343"
        echo "  - Testing Agent:   http://localhost:8344"
        echo ""
        echo "WebSocket Endpoints:"
        echo "  - Project Manager: ws://localhost:8342/ws"
        echo "  - Coding Agent:    ws://localhost:8343/ws"
        echo "  - Testing Agent:   ws://localhost:8344/ws"
        echo ""
        show_logs
        echo ""
        print_status "INFO" "To stop all agents, run: $0 stop"
        echo ""
    else
        print_status "ERROR" "Failed to start all agents"
        stop_agents
        exit 1
    fi
}

# Set up trap to clean up on exit (only for stop command)
if [ "${1:-}" = "stop" ] || [ "${1:-}" = "restart" ]; then
    trap stop_agents EXIT INT TERM
fi

# Run main function
main "$@"