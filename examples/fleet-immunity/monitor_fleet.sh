#!/bin/bash
# Monitor fleet status in real-time

echo ""
echo "  ━━━━━━ FLEET MONITOR ━━━━━━"
echo ""
echo "  Watching rover logs..."
echo "  Press Ctrl+C to exit"
echo ""

cd "$(dirname "$0")"

# Use tail to follow all rover logs
tail -f logs/alpha.log logs/beta.log logs/gamma.log 2>/dev/null || \
    echo "[INFO] No logs yet. Start the fleet with ./launch_fleet.sh"
