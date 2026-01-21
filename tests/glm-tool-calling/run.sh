#!/bin/bash
# GLM-4.7-Flash Tool Calling Example
#
# This example validates GLM-4.7-Flash tool calling capabilities:
# - Python-style parsing: function_name(arg="value")
# - XML parsing: <tool_call>...</tool_call>
# - Thinking block handling: <|thought|>...<|/thought|>
#
# Requirements:
# - 32GB+ RAM for GLM-4.7-Flash (or 24GB with --kv-quant)
# - Model auto-downloads from HuggingFace on first use
#
# Usage:
#   ./run.sh              # Run all tool tests
#   ./run.sh --verbose    # Show debug output
#   ./run.sh --single N   # Run only task N (0-indexed)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${BINARY:-$SCRIPT_DIR/../../target/debug/arkavo}"
TASKS_FILE="$SCRIPT_DIR/tasks.json"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse arguments
VERBOSE=false
SINGLE_TASK=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --single|-s)
            SINGLE_TASK="$2"
            shift 2
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            exit 1
            ;;
    esac
done

print_banner() {
    echo -e "${BLUE}"
    echo "================================================================"
    echo "     GLM-4.7-Flash Tool Calling Validation"
    echo "================================================================"
    echo -e "${NC}"
    echo ""
    echo "This example tests GLM-4.7-Flash's ability to:"
    echo "  - Parse tool schemas from system prompt"
    echo "  - Generate tool calls in Python-style or XML format"
    echo "  - Handle observation results and continue reasoning"
    echo ""
}

check_prerequisites() {
    echo -e "${CYAN}Checking prerequisites...${NC}"

    # Check binary
    if [[ ! -f "$BINARY" ]]; then
        echo -e "${RED}Error: Arkavo binary not found at $BINARY${NC}"
        echo "Run: cargo build"
        exit 1
    fi
    echo -e "  ${GREEN}✓${NC} Arkavo binary found"

    # Check jq
    if ! command -v jq &> /dev/null; then
        echo -e "${RED}Error: jq not found${NC}"
        echo "Install: brew install jq (macOS) or apt install jq (Linux)"
        exit 1
    fi
    echo -e "  ${GREEN}✓${NC} jq installed"

    # Check RAM (GLM requires 24GB+)
    local ram_gb
    if [[ "$(uname)" == "Darwin" ]]; then
        ram_gb=$(sysctl -n hw.memsize | awk '{print int($1/1073741824)}')
    else
        ram_gb=$(grep MemTotal /proc/meminfo | awk '{print int($2/1048576)}')
    fi

    if [[ $ram_gb -lt 24 ]]; then
        echo -e "${YELLOW}Warning: GLM-4.7-Flash requires 24GB+ RAM (detected: ${ram_gb}GB)${NC}"
        echo "The model may not load or run slowly."
    else
        echo -e "  ${GREEN}✓${NC} RAM: ${ram_gb}GB (GLM requires 24GB+)"
    fi

    echo ""
}

run_task() {
    local task_id="$1"
    local task_desc="$2"

    echo -e "${YELLOW}Task: [$task_id]${NC}"
    echo -e "  ${CYAN}$task_desc${NC}"
    echo ""

    # Build command
    local cmd=("$BINARY" chat --model glm --repo-context off --prompt "$task_desc")

    if $VERBOSE; then
        cmd+=(--debug)
        export ARKAVO_DEBUG=1
        export ARKAVO_DEBUG_CHAT=1
    fi

    # Run command
    echo -e "${BLUE}Response:${NC}"
    "${cmd[@]}" 2>&1 || {
        echo -e "${RED}Task failed${NC}"
        return 1
    }

    echo ""
    echo -e "${GREEN}---${NC}"
    echo ""
}

run_all_tasks() {
    local task_count
    task_count=$(jq '.tasks | length' "$TASKS_FILE")

    echo -e "${CYAN}Running $task_count tool calling tasks with GLM-4.7-Flash...${NC}"
    echo ""

    local start=0
    local end=$((task_count - 1))

    # If single task specified, only run that one
    if [[ -n "$SINGLE_TASK" ]]; then
        if [[ "$SINGLE_TASK" -ge 0 && "$SINGLE_TASK" -lt "$task_count" ]]; then
            start=$SINGLE_TASK
            end=$SINGLE_TASK
        else
            echo -e "${RED}Invalid task index: $SINGLE_TASK (0-$((task_count-1)) available)${NC}"
            exit 1
        fi
    fi

    for i in $(seq $start $end); do
        local task_id
        local task_desc
        task_id=$(jq -r ".tasks[$i].id" "$TASKS_FILE")
        task_desc=$(jq -r ".tasks[$i].description" "$TASKS_FILE")

        run_task "$task_id" "$task_desc"

        # Brief pause between tasks
        if [[ $i -lt $end ]]; then
            sleep 1
        fi
    done
}

# Main
print_banner
check_prerequisites
run_all_tasks

echo -e "${GREEN}GLM tool calling validation complete!${NC}"
