#!/bin/bash
# Run tasks for a scenario

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${BINARY:-$SCRIPT_DIR/../target/debug/arkavo}"
SCENARIO="$1"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

list_scenarios() {
    echo "Available scenarios:"
    for dir in "$SCRIPT_DIR"/*/; do
        local name=$(basename "$dir")
        if [ -f "$dir/tasks.json" ]; then
            local count=$(jq '.tasks | length' "$dir/tasks.json" 2>/dev/null || echo 0)
            echo "  $name ($count tasks)"
        fi
    done
}

if [ -z "$SCENARIO" ]; then
    echo "Usage: $0 <scenario>"
    echo ""
    list_scenarios
    exit 1
fi

SCENARIO_DIR="$SCRIPT_DIR/$SCENARIO"
TASKS_FILE="$SCENARIO_DIR/tasks.json"

if [ ! -f "$TASKS_FILE" ]; then
    echo -e "${RED}No tasks.json found for $SCENARIO${NC}"
    echo ""
    list_scenarios
    exit 1
fi

echo -e "${GREEN}Running scenario: $SCENARIO${NC}"
echo ""

# Run each task
task_count=$(jq '.tasks | length' "$TASKS_FILE")

for i in $(seq 0 $((task_count - 1))); do
    task_id=$(jq -r ".tasks[$i].id" "$TASKS_FILE")
    task_desc=$(jq -r ".tasks[$i].description" "$TASKS_FILE")

    echo -e "${YELLOW}Task $task_id:${NC} $task_desc"
    echo ""

    "$BINARY" task --yes "$task_desc" 2>&1 || true

    echo ""
    echo "---"
    sleep 2
done

echo -e "${GREEN}Scenario complete.${NC}"
