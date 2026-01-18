#!/bin/bash

# Claude Code Agent Example Runner
# Demonstrates various Claude Code capabilities

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
AGENT_URL="http://localhost:8345"
WORKSPACE_DIR="./workspace"

# Ensure workspace exists
mkdir -p "$WORKSPACE_DIR"

print_header() {
    echo ""
    echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
    echo ""
}

print_status() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

check_agent() {
    if ! curl -s "$AGENT_URL/.well-known/agent.json" > /dev/null 2>&1; then
        print_error "Agent not running. Please start it first:"
        echo "  ./launch_agent.sh start"
        exit 1
    fi
    print_status "Agent is running ✓"
}

# Example: Generate a simple function
example_generate() {
    print_header "Example 1: Generate a Calculator Class"
    
    REQUEST='{
        "tool": "claude_code_run",
        "prompt": "Create a Calculator class in Python with methods for add, subtract, multiply, and divide. Include error handling for division by zero. Save it as calculator.py in the workspace.",
        "context": {
            "workspace": "'$WORKSPACE_DIR'"
        }
    }'
    
    print_status "Sending request to generate Calculator class..."
    curl -X POST "$AGENT_URL/v1/agent/task" \
        -H "Content-Type: application/json" \
        -d "$REQUEST" \
        -w "\n"
    
    sleep 2
    
    if [ -f "$WORKSPACE_DIR/calculator.py" ]; then
        print_status "Generated file: $WORKSPACE_DIR/calculator.py"
        echo ""
        echo "Content:"
        cat "$WORKSPACE_DIR/calculator.py"
    fi
}

# Example: Analyze existing code
example_analyze() {
    print_header "Example 2: Analyze and Improve Code"
    
    # First, ensure there's code to analyze
    if [ ! -f "$WORKSPACE_DIR/calculator.py" ]; then
        print_status "Creating sample code for analysis..."
        cat > "$WORKSPACE_DIR/calculator.py" << 'EOF'
class Calculator:
    def add(self, a, b):
        return a + b
    
    def subtract(self, a, b):
        return a - b
    
    def multiply(self, a, b):
        return a * b
    
    def divide(self, a, b):
        if b == 0:
            raise ValueError("Cannot divide by zero")
        return a / b
EOF
    fi
    
    REQUEST='{
        "tool": "claude_code_plan",
        "prompt": "Analyze the calculator.py file in the workspace. Suggest improvements for code quality, add type hints, improve error handling, and recommend unit tests.",
        "context": {
            "workspace": "'$WORKSPACE_DIR'"
        }
    }'
    
    print_status "Sending request to analyze code..."
    curl -X POST "$AGENT_URL/v1/agent/task" \
        -H "Content-Type: application/json" \
        -d "$REQUEST" \
        -w "\n"
}

# Example: Generate tests
example_tests() {
    print_header "Example 3: Generate Unit Tests"
    
    REQUEST='{
        "tool": "claude_code_run",
        "prompt": "Generate comprehensive unit tests for the Calculator class in calculator.py. Use pytest framework. Save the tests as test_calculator.py.",
        "context": {
            "workspace": "'$WORKSPACE_DIR'"
        }
    }'
    
    print_status "Sending request to generate tests..."
    curl -X POST "$AGENT_URL/v1/agent/task" \
        -H "Content-Type: application/json" \
        -d "$REQUEST" \
        -w "\n"
    
    sleep 2
    
    if [ -f "$WORKSPACE_DIR/test_calculator.py" ]; then
        print_status "Generated test file: $WORKSPACE_DIR/test_calculator.py"
        echo ""
        echo "Content preview:"
        head -20 "$WORKSPACE_DIR/test_calculator.py"
    fi
}

# Example: Scaffold a project
example_scaffold() {
    print_header "Example 4: Scaffold a REST API Project"
    
    REQUEST='{
        "tool": "claude_code_run",
        "prompt": "Create a simple REST API project structure for a todo list application. Include: 1) Main application file (app.py), 2) Models (models.py), 3) Routes (routes.py), 4) Requirements file (requirements.txt), 5) README with setup instructions. Use Flask or FastAPI.",
        "context": {
            "workspace": "'$WORKSPACE_DIR'",
            "project_name": "todo_api"
        }
    }'
    
    print_status "Sending request to scaffold project..."
    curl -X POST "$AGENT_URL/v1/agent/task" \
        -H "Content-Type: application/json" \
        -d "$REQUEST" \
        -w "\n"
    
    sleep 3
    
    print_status "Project structure created:"
    ls -la "$WORKSPACE_DIR/" 2>/dev/null || echo "Check workspace directory"
}

# Example: Interactive coding session
example_interactive() {
    print_header "Example 5: Interactive Coding Session"
    
    print_status "Starting interactive session..."
    echo "You can now send coding requests to the agent."
    echo "Example prompts:"
    echo "  - 'Add a new method to Calculator for calculating power'"
    echo "  - 'Refactor the code to use a base class'"
    echo "  - 'Add logging to all methods'"
    echo ""
    echo "Type 'quit' to exit"
    echo ""
    
    while true; do
        read -p "Enter your coding task (or 'quit'): " task
        
        if [ "$task" = "quit" ]; then
            break
        fi
        
        if [ -z "$task" ]; then
            continue
        fi
        
        REQUEST="{
            \"tool\": \"claude_code_run\",
            \"prompt\": \"$task\",
            \"context\": {
                \"workspace\": \"$WORKSPACE_DIR\"
            }
        }"
        
        curl -X POST "$AGENT_URL/v1/agent/task" \
            -H "Content-Type: application/json" \
            -d "$REQUEST" \
            -w "\n"
        
        echo ""
    done
}

# Example: Code review
example_review() {
    print_header "Example 6: Code Review"
    
    # Create a sample file with intentional issues
    cat > "$WORKSPACE_DIR/sample_code.py" << 'EOF'
def process_data(data):
    result = []
    for i in range(len(data)):
        if data[i] > 0:
            result.append(data[i] * 2)
    return result

def calculate_average(numbers):
    total = 0
    count = 0
    for n in numbers:
        total = total + n
        count = count + 1
    return total / count

class DataProcessor:
    def __init__(self):
        self.data = []
    
    def add_data(self, item):
        self.data.append(item)
    
    def process(self):
        processed = []
        for item in self.data:
            processed.append(item.upper())
        return processed
EOF
    
    REQUEST='{
        "tool": "claude_code_run",
        "prompt": "Review the code in sample_code.py. Identify issues, suggest improvements, and refactor the code to be more Pythonic and efficient. Save the improved version as sample_code_improved.py.",
        "context": {
            "workspace": "'$WORKSPACE_DIR'"
        }
    }'
    
    print_status "Sending request for code review..."
    curl -X POST "$AGENT_URL/v1/agent/task" \
        -H "Content-Type: application/json" \
        -d "$REQUEST" \
        -w "\n"
}

# Main menu
show_menu() {
    print_header "Claude Code Agent Examples"
    echo "Select an example to run:"
    echo ""
    echo "  1) Generate - Create a Calculator class"
    echo "  2) Analyze  - Analyze and improve existing code"
    echo "  3) Tests    - Generate unit tests"
    echo "  4) Scaffold - Create a REST API project structure"
    echo "  5) Review   - Perform code review and refactoring"
    echo "  6) Interactive - Start interactive coding session"
    echo ""
    echo "  a) All     - Run all examples (except interactive)"
    echo "  q) Quit    - Exit"
    echo ""
}

# Process command line argument or show menu
case "${1:-menu}" in
    generate)
        check_agent
        example_generate
        ;;
    analyze)
        check_agent
        example_analyze
        ;;
    tests)
        check_agent
        example_tests
        ;;
    scaffold)
        check_agent
        example_scaffold
        ;;
    review)
        check_agent
        example_review
        ;;
    interactive)
        check_agent
        example_interactive
        ;;
    all)
        check_agent
        example_generate
        sleep 2
        example_analyze
        sleep 2
        example_tests
        sleep 2
        example_review
        sleep 2
        example_scaffold
        print_header "All Examples Completed"
        print_status "Check the workspace directory for generated files:"
        ls -la "$WORKSPACE_DIR/"
        ;;
    menu)
        while true; do
            show_menu
            read -p "Enter your choice: " choice
            
            case $choice in
                1) check_agent; example_generate ;;
                2) check_agent; example_analyze ;;
                3) check_agent; example_tests ;;
                4) check_agent; example_scaffold ;;
                5) check_agent; example_review ;;
                6) check_agent; example_interactive ;;
                a|A) 
                    check_agent
                    example_generate
                    sleep 2
                    example_analyze
                    sleep 2
                    example_tests
                    sleep 2
                    example_review
                    sleep 2
                    example_scaffold
                    ;;
                q|Q) exit 0 ;;
                *) print_error "Invalid choice" ;;
            esac
            
            echo ""
            read -p "Press Enter to continue..."
        done
        ;;
    *)
        echo "Usage: $0 {generate|analyze|tests|scaffold|review|interactive|all|menu}"
        echo ""
        echo "Examples:"
        echo "  $0 generate    - Generate a Calculator class"
        echo "  $0 analyze     - Analyze existing code"
        echo "  $0 tests       - Generate unit tests"
        echo "  $0 scaffold    - Scaffold a project"
        echo "  $0 review      - Review and refactor code"
        echo "  $0 interactive - Interactive coding session"
        echo "  $0 all         - Run all examples"
        echo "  $0             - Show interactive menu"
        exit 1
        ;;
esac