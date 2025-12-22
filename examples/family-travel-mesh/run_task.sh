#!/bin/bash
# Family Travel Mesh - Run HRM Task
# Uses arkavo chat to demonstrate HRM orchestration

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${PROJECT_ROOT}/target/debug/arkavo"

if [ ! -f "$BINARY" ]; then
    echo "Binary not found. Run: cargo build"
    exit 1
fi

PROMPT="${1:-Plan a Friday afternoon in Las Vegas for a family with twin 3-year-old toddlers. Budget is \$200, time window 12:00-18:00. Use the hrm_create_task tool to create an orchestrated task.}"

echo ""
echo "═══════════════════════════════════════════════════════════════════════"
echo "   HRM TASK DEMO                                                       "
echo "═══════════════════════════════════════════════════════════════════════"
echo ""
echo "Prompt: $PROMPT"
echo ""
echo "───────────────────────────────────────────────────────────────────────"
echo ""

cd "$SCRIPT_DIR/agents/conductor"
timeout 120 "$BINARY" chat --prompt "$PROMPT" 2>&1 || true

echo ""
echo "═══════════════════════════════════════════════════════════════════════"
echo ""
