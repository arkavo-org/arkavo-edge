#!/bin/bash
# RimWorld Mesh Monitor - checks every 5 minutes for progress/stuck agents
LOG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/logs"
MONITOR_LOG="$LOG_DIR/monitor.log"
PREV_TICK=0
STUCK_COUNT=0

check() {
    local ts=$(date '+%H:%M:%S')
    local tick=$(grep "tick=" "$LOG_DIR/commander.log" 2>/dev/null | tail -1 | grep -oE 'tick=[0-9]+' | cut -d= -f2)
    local episodes=$(grep -c "Episode synthesized" "$LOG_DIR/commander.log" 2>/dev/null || echo 0)
    local tool_ok=$(grep -c "Tool.*succeeded" "$LOG_DIR/commander.log" 2>/dev/null || echo 0)
    local cooldowns=$(grep -c "cooldown" "$LOG_DIR/commander.log" 2>/dev/null || echo 0)
    local lessons=$(grep -c "Lesson added" "$LOG_DIR/commander.log" 2>/dev/null || echo 0)

    # Health checks
    local h_cmd=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:8401/health 2>/dev/null)
    local h_sur=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:8410/health 2>/dev/null)
    local h_ind=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:8411/health 2>/dev/null)
    local h_def=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:8412/health 2>/dev/null)

    local health="cmd=$h_cmd sur=$h_sur ind=$h_ind def=$h_def"
    local dead=""
    [[ "$h_cmd" != "200" ]] && dead="$dead COMMANDER"
    [[ "$h_sur" != "200" ]] && dead="$dead SURVIVAL"
    [[ "$h_ind" != "200" ]] && dead="$dead INDUSTRY"
    [[ "$h_def" != "200" ]] && dead="$dead DEFENSE"

    # Stuck detection
    local status="OK"
    if [[ -n "$dead" ]]; then
        status="DEAD:$dead"
    elif [[ "$tick" -le "$PREV_TICK" && "$PREV_TICK" -gt 0 ]]; then
        STUCK_COUNT=$((STUCK_COUNT + 1))
        if [[ "$STUCK_COUNT" -ge 2 ]]; then
            status="STUCK (tick unchanged x${STUCK_COUNT})"
        else
            status="WARN (tick unchanged)"
        fi
    else
        STUCK_COUNT=0
    fi

    printf "[%s] tick=%-8s episodes=%-4s tools_ok=%-4s cooldowns=%-4s lessons=%-4s | %s | %s\n" \
        "$ts" "${tick:-?}" "$episodes" "$tool_ok" "$cooldowns" "$lessons" "$health" "$status"

    PREV_TICK=${tick:-0}
}

echo "=== RimWorld Mesh Monitor (5min interval) ==="
echo "Logging to: $MONITOR_LOG"
echo ""

while true; do
    line=$(check)
    echo "$line"
    echo "$line" >> "$MONITOR_LOG"
    sleep 300
done
