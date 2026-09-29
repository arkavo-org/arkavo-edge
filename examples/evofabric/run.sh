#!/bin/bash
# EvoFabric - Full agent pipeline with local model
#
# The agent reads the target file, asks the local model to propose an OpBundle,
# applies it to the AST, verifies in an isolated workspace, and commits.
#
# Usage:
#   ./run.sh                                    # Default: add #[inline] to is_small_model
#   ./run.sh "add #[must_use] to process"       # Custom instruction

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/../.."
# Resolved as: $BINARY if set, then the source build, then arkavo on PATH.
source "${SCRIPT_DIR}/../common/resolve_binary.sh"
resolve_arkavo_binary || true

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# Default task
INSTRUCTION="${1:-add #[inline] to is_small_model}"
TARGET="crates/arkavo-llm/src/llamacpp_provider.rs"
TASK="[evofabric] ${TARGET} ${INSTRUCTION}"

echo -e "${BOLD}EvoFabric Agent Pipeline${NC}"
echo "========================"
echo ""

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    explain_missing_arkavo_binary
    exit 1
fi

echo -e "${CYAN}Target:${NC}      ${TARGET}"
echo -e "${CYAN}Instruction:${NC} ${INSTRUCTION}"
echo -e "${CYAN}Full task:${NC}   ${TASK}"
echo ""

echo -e "${BOLD}--- Starting EvoFabric Agent ---${NC}"
echo ""

cd "$REPO_ROOT"
"$BINARY" chat --prompt "$TASK"

echo ""

# Show what happened
LAST_COMMIT=$(git log --oneline -1)
if echo "$LAST_COMMIT" | grep -q "evofabric:"; then
    echo -e "${GREEN}EvoFabric commit created:${NC}"
    echo "  $LAST_COMMIT"
    echo ""
    echo "View the change:"
    echo "  git diff HEAD~1"
    echo ""
    echo "Revert if needed:"
    echo "  git revert HEAD"
else
    echo -e "${CYAN}No evofabric commit was created.${NC}"
    echo "This could mean:"
    echo "  - The model proposed an invalid OpBundle (check output above)"
    echo "  - Compilation failed in the temp workspace"
    echo "  - Tests failed after applying the change"
fi
