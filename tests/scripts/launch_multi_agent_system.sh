#!/bin/bash

# Launch Multi-Agent System
# This script starts all 11 specialized agents for the knowledge sharing system

set -e

echo "🚀 Launching Arkavo Multi-Agent System..."
echo "========================================"

# Build the project first
echo "📦 Building arkavo..."
cargo build --release

# Array of agent directories
agents=(
    "orchestrator"
    "security"
    "code-review"
    "database"
    "testing"
    "documentation"
    "performance"
    "devops"
    "frontend"
    "architecture"
    "data-science"
)

# Start each agent in the background
pids=()
for agent in "${agents[@]}"; do
    echo "🤖 Starting $agent agent..."
    cd "examples/software-development-lifecycle/$agent"
    ../../target/release/arkavo agent run > "$agent.log" 2>&1 &
    pid=$!
    pids+=($pid)
    echo "   ✓ Started $agent agent (PID: $pid)"
    cd ../..
    sleep 1  # Give each agent time to start
done

echo ""
echo "✅ All agents started successfully!"
echo ""
echo "📊 Agent Status:"
echo "----------------"
for i in "${!agents[@]}"; do
    echo "- ${agents[$i]}: PID ${pids[$i]}"
done

echo ""
echo "🌐 Agent Endpoints:"
echo "-------------------"
echo "- Orchestrator:    http://localhost:8342"
echo "- Security:        http://localhost:8343"
echo "- Code Review:     http://localhost:8344"
echo "- Database:        http://localhost:8345"
echo "- Testing:         http://localhost:8346"
echo "- Documentation:   http://localhost:8347"
echo "- Performance:     http://localhost:8348"
echo "- DevOps:          http://localhost:8349"
echo "- Frontend:        http://localhost:8350"
echo "- Architecture:    http://localhost:8351"
echo "- Data Science:    http://localhost:8352"

echo ""
echo "📝 Logs are being written to examples/software-development-lifecycle/*/[agent-name].log"
echo ""
echo "🛑 Press Ctrl+C to stop all agents..."

# Function to cleanup on exit
cleanup() {
    echo ""
    echo "🛑 Stopping all agents..."
    for pid in "${pids[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid"
            echo "   ✓ Stopped PID $pid"
        fi
    done
    echo "👋 Goodbye!"
    exit 0
}

# Set up trap to cleanup on Ctrl+C
trap cleanup SIGINT SIGTERM

# Wait indefinitely
while true; do
    sleep 1
done