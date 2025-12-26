#!/bin/bash
# Monitor learning events across all rovers in the fleet
#
# Shows the progression:
#   Observation → Episode → Lesson → Gossip → Policy Cache
#
# Usage: ./monitor_learning.sh

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo ""
echo "  ╔═══════════════════════════════════════════════════════════════╗"
echo "  ║                 LEARNING MONITOR                              ║"
echo "  ║                                                               ║"
echo "  ║  Watching: Observation → Episode → Lesson → Gossip → Policy  ║"
echo "  ╚═══════════════════════════════════════════════════════════════╝"
echo ""
echo "  Press Ctrl+C to stop monitoring"
echo ""

# Keywords to filter for learning-related events
# Using extended grep to match any of these patterns
tail -f logs/*.log 2>/dev/null | grep --line-buffered -E \
    "Observation (buffered|threshold)|Episode (synthesis|synthesized|added)|Lesson (synthesis|pattern|announcement|vote|approved|rejected|added)|policy cache|Starting (episode|lesson) synthesis|consensus"
