#!/bin/bash

# Agent Collaboration Demo Scenario
# This script demonstrates agent-to-agent communication

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PM_URL="http://localhost:8342"
CODING_URL="http://localhost:8343"
TESTING_URL="http://localhost:8344"

# Function to print colored output
print_status() {
    local status=$1
    local message=$2
    case $status in
        "PM")
            echo -e "${BLUE}[PROJECT MANAGER]${NC} $message"
            ;;
        "CODING")
            echo -e "${GREEN}[CODING AGENT]${NC} $message"
            ;;
        "TESTING")
            echo -e "${MAGENTA}[TESTING AGENT]${NC} $message"
            ;;
        "INFO")
            echo -e "${CYAN}[INFO]${NC} $message"
            ;;
        "SUCCESS")
            echo -e "${GREEN}[SUCCESS]${NC} $message"
            ;;
        "ERROR")
            echo -e "${RED}[ERROR]${NC} $message"
            ;;
    esac
}

# Function to make RPC call
rpc_call() {
    local url=$1
    local method=$2
    local params=$3
    local id=${4:-1}
    
    curl -s -X POST "$url/rpc" \
        -H "Content-Type: application/json" \
        -d "{
            \"jsonrpc\": \"2.0\",
            \"method\": \"$method\",
            \"params\": $params,
            \"id\": $id
        }"
}

# Function to check agent health
check_agents() {
    print_status "INFO" "Checking agent health..."
    
    local all_healthy=true
    
    if curl -s "$PM_URL/health" > /dev/null 2>&1; then
        print_status "SUCCESS" "Project Manager is ready"
    else
        print_status "ERROR" "Project Manager is not responding"
        all_healthy=false
    fi
    
    if curl -s "$CODING_URL/health" > /dev/null 2>&1; then
        print_status "SUCCESS" "Coding Agent is ready"
    else
        print_status "ERROR" "Coding Agent is not responding"
        all_healthy=false
    fi
    
    if curl -s "$TESTING_URL/health" > /dev/null 2>&1; then
        print_status "SUCCESS" "Testing Agent is ready"
    else
        print_status "ERROR" "Testing Agent is not responding"
        all_healthy=false
    fi
    
    if [ "$all_healthy" = false ]; then
        print_status "ERROR" "Not all agents are running. Please run: ./launch_agents.sh"
        exit 1
    fi
}

# Demo Scenario 1: Agent Discovery
scenario_discovery() {
    echo ""
    echo "=== Scenario 1: Agent Discovery ==="
    echo ""
    
    print_status "INFO" "Each agent will broadcast its capabilities..."
    
    # Project Manager broadcasts
    print_status "PM" "Broadcasting capabilities"
    rpc_call "$PM_URL" "agent_broadcast" '{
        "broadcast": {
            "agent_id": "project-manager",
            "broadcast_type": "capabilities",
            "capabilities": [
                {
                    "name": "task_decomposition",
                    "description": "Break down complex tasks into subtasks"
                },
                {
                    "name": "agent_coordination",
                    "description": "Coordinate between multiple agents"
                }
            ]
        }
    }' 1 > /dev/null
    
    # Coding Agent broadcasts
    print_status "CODING" "Broadcasting capabilities"
    rpc_call "$CODING_URL" "agent_broadcast" '{
        "broadcast": {
            "agent_id": "coding-agent",
            "broadcast_type": "capabilities",
            "capabilities": [
                {
                    "name": "code_implementation",
                    "description": "Implement code based on requirements"
                },
                {
                    "name": "refactoring",
                    "description": "Refactor existing code"
                }
            ]
        }
    }' 2 > /dev/null
    
    # Testing Agent broadcasts
    print_status "TESTING" "Broadcasting capabilities"
    rpc_call "$TESTING_URL" "agent_broadcast" '{
        "broadcast": {
            "agent_id": "testing-agent",
            "broadcast_type": "capabilities",
            "capabilities": [
                {
                    "name": "test_generation",
                    "description": "Generate unit and integration tests"
                },
                {
                    "name": "coverage_analysis",
                    "description": "Analyze test coverage"
                }
            ]
        }
    }' 3 > /dev/null
    
    print_status "SUCCESS" "All agents have broadcast their capabilities"
    
    # Project Manager discovers other agents
    print_status "PM" "Discovering available agents..."
    response=$(rpc_call "$PM_URL" "agent_discover" '{"filter": null}' 4)
    echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
}

# Demo Scenario 2: Direct Agent Query
scenario_query() {
    echo ""
    echo "=== Scenario 2: Direct Agent Query ==="
    echo ""
    
    print_status "INFO" "Project Manager queries Coding Agent about its status..."
    
    print_status "PM" "Sending query to Coding Agent"
    response=$(rpc_call "$PM_URL" "agent_query" '{
        "request": {
            "from_agent_id": "project-manager",
            "to_agent_id": "coding-agent",
            "query": "What is your current workload and availability?",
            "context": {
                "priority": "normal"
            }
        }
    }' 5)
    
    print_status "CODING" "Response received"
    echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
}

# Demo Scenario 3: Task Assignment
scenario_task_assignment() {
    echo ""
    echo "=== Scenario 3: Task Assignment Workflow ==="
    echo ""
    
    print_status "INFO" "Project Manager assigns a coding task..."
    
    # Step 1: PM sends task to Coding Agent
    print_status "PM" "Assigning task to Coding Agent: Implement Calculator class"
    
    task_response=$(rpc_call "$PM_URL" "message/send" '{
        "request": {
            "message": {
                "from_agent": "project-manager",
                "to_agent": "coding-agent",
                "content": "Please implement a Calculator class with add and subtract methods",
                "message_type": "task_assignment",
                "metadata": {
                    "task_id": "calc-001",
                    "priority": "high",
                    "deadline": "2024-01-20T12:00:00Z"
                }
            }
        }
    }' 6)
    
    print_status "CODING" "Task received and acknowledged"
    
    # Step 2: Coding Agent implements and sends to Testing
    print_status "CODING" "Implementing Calculator class..."
    sleep 2
    
    print_status "CODING" "Sending implementation to Testing Agent"
    
    test_request=$(rpc_call "$CODING_URL" "message/send" '{
        "request": {
            "message": {
                "from_agent": "coding-agent",
                "to_agent": "testing-agent",
                "content": "Please test the following Calculator implementation",
                "message_type": "test_request",
                "metadata": {
                    "task_id": "calc-001",
                    "code": "class Calculator:\\n    def add(self, a, b):\\n        return a + b\\n    \\n    def subtract(self, a, b):\\n        return a - b"
                }
            }
        }
    }' 7)
    
    print_status "TESTING" "Code received for testing"
    
    # Step 3: Testing Agent validates and reports back
    print_status "TESTING" "Generating and running tests..."
    sleep 2
    
    print_status "TESTING" "Sending test results to Project Manager"
    
    test_results=$(rpc_call "$TESTING_URL" "message/send" '{
        "request": {
            "message": {
                "from_agent": "testing-agent",
                "to_agent": "project-manager",
                "content": "Test results for Calculator implementation",
                "message_type": "test_results",
                "metadata": {
                    "task_id": "calc-001",
                    "tests_passed": 4,
                    "tests_failed": 0,
                    "coverage": 100,
                    "status": "success"
                }
            }
        }
    }' 8)
    
    print_status "PM" "Test results received"
    print_status "SUCCESS" "Task completed successfully with 100% test coverage"
}

# Demo Scenario 4: Chat Session
scenario_chat() {
    echo ""
    echo "=== Scenario 4: Interactive Chat Session ==="
    echo ""
    
    print_status "INFO" "Opening chat session between agents..."
    
    # Open chat session
    print_status "PM" "Opening chat session"
    chat_response=$(rpc_call "$PM_URL" "chat_open" '{
        "request": {
            "agent_id": "project-manager",
            "participants": ["coding-agent", "testing-agent"],
            "topic": "Sprint Planning"
        }
    }' 9)
    
    session_id=$(echo "$chat_response" | python3 -c "import sys, json; print(json.load(sys.stdin).get('result', {}).get('session_id', 'unknown'))" 2>/dev/null || echo "session-001")
    
    print_status "SUCCESS" "Chat session opened: $session_id"
    
    # Send messages in the session
    print_status "PM" "What features should we prioritize this sprint?"
    
    rpc_call "$PM_URL" "chat_send" "{
        \"session_id\": \"$session_id\",
        \"message\": {
            \"role\": \"project-manager\",
            \"content\": \"What features should we prioritize this sprint?\"
        }
    }" 10 > /dev/null
    
    sleep 1
    
    print_status "CODING" "I suggest implementing the authentication module first"
    print_status "TESTING" "Agreed, and we should include security testing"
    
    print_status "SUCCESS" "Chat session demonstration complete"
}

# Demo Scenario 5: Concurrent Tasks
scenario_concurrent() {
    echo ""
    echo "=== Scenario 5: Concurrent Task Handling ==="
    echo ""
    
    print_status "INFO" "Project Manager assigns multiple tasks concurrently..."
    
    # Assign multiple tasks
    print_status "PM" "Assigning Task 1: API endpoint implementation"
    rpc_call "$PM_URL" "message/send" '{
        "request": {
            "message": {
                "from_agent": "project-manager",
                "to_agent": "coding-agent",
                "content": "Implement REST API endpoints",
                "message_type": "task_assignment",
                "metadata": {"task_id": "api-001"}
            }
        }
    }' 11 > /dev/null &
    
    print_status "PM" "Assigning Task 2: Database schema design"
    rpc_call "$PM_URL" "message/send" '{
        "request": {
            "message": {
                "from_agent": "project-manager",
                "to_agent": "coding-agent",
                "content": "Design database schema",
                "message_type": "task_assignment",
                "metadata": {"task_id": "db-001"}
            }
        }
    }' 12 > /dev/null &
    
    print_status "PM" "Assigning Task 3: Test suite preparation"
    rpc_call "$PM_URL" "message/send" '{
        "request": {
            "message": {
                "from_agent": "project-manager",
                "to_agent": "testing-agent",
                "content": "Prepare integration test suite",
                "message_type": "task_assignment",
                "metadata": {"task_id": "test-001"}
            }
        }
    }' 13 > /dev/null &
    
    wait
    
    print_status "SUCCESS" "All concurrent tasks assigned successfully"
}

# Main execution
main() {
    echo "============================================"
    echo "Agent Collaboration Communication Demo"
    echo "============================================"
    echo ""
    
    # Check if agents are running
    check_agents
    
    echo ""
    print_status "INFO" "Starting demonstration scenarios..."
    print_status "INFO" "Note: Monitor the AGUI dashboard at http://localhost:3000 for real-time visualization"
    echo ""
    
    # Run scenarios based on argument
    case "${1:-all}" in
        discovery)
            scenario_discovery
            ;;
        query)
            scenario_query
            ;;
        task)
            scenario_task_assignment
            ;;
        chat)
            scenario_chat
            ;;
        concurrent)
            scenario_concurrent
            ;;
        all)
            scenario_discovery
            sleep 2
            scenario_query
            sleep 2
            scenario_task_assignment
            sleep 2
            scenario_chat
            sleep 2
            scenario_concurrent
            ;;
        help|--help|-h)
            echo "Usage: $0 [scenario]"
            echo ""
            echo "Scenarios:"
            echo "  discovery   - Agent capability discovery"
            echo "  query       - Direct agent-to-agent query"
            echo "  task        - Task assignment workflow"
            echo "  chat        - Interactive chat session"
            echo "  concurrent  - Concurrent task handling"
            echo "  all         - Run all scenarios (default)"
            echo "  help        - Show this help message"
            exit 0
            ;;
        *)
            print_status "ERROR" "Unknown scenario: $1"
            echo "Run '$0 help' for usage information"
            exit 1
            ;;
    esac
    
    echo ""
    print_status "SUCCESS" "Demonstration complete!"
    echo ""
    print_status "INFO" "View the AGUI dashboard for detailed monitoring:"
    echo "  arkavo ui"
    echo ""
    print_status "INFO" "Agent logs available at:"
    echo "  ./logs/project-manager.log"
    echo "  ./logs/coding-agent.log"
    echo "  ./logs/testing-agent.log"
    echo ""
}

# Run main function
main "$@"