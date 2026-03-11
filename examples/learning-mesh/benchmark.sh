#!/usr/bin/env bash
set -euo pipefail

# Multi-round benchmark for the learning-mesh agent swarm.
# Submits tasks from tasks.json across multiple rounds, collects quality
# scores via the AG-UI WebSocket, and produces a JSON report with a
# PASS/FAIL verdict based on average quality and improvement slope.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GATEWAY="${GATEWAY_URL:-ws://localhost:7700/ws}"
TASKS_FILE="$SCRIPT_DIR/tasks.json"
ROUNDS="${ROUNDS:-3}"
REPORT="$SCRIPT_DIR/benchmark-report.json"
WAIT_BETWEEN_TASKS="${WAIT_BETWEEN_TASKS:-30}"
WAIT_BETWEEN_ROUNDS="${WAIT_BETWEEN_ROUNDS:-15}"

# Check dependencies
for cmd in jq websocat; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "Error: $cmd is required but not installed."
        exit 1
    fi
done

if [[ ! -f "$TASKS_FILE" ]]; then
    echo "Error: tasks.json not found at $TASKS_FILE"
    exit 1
fi

NUM_TASKS=$(jq '.tasks | length' "$TASKS_FILE")
echo "=== Learning Mesh Benchmark ==="
echo "Gateway: $GATEWAY"
echo "Tasks:   $NUM_TASKS per round"
echo "Rounds:  $ROUNDS"
echo ""

# Collect quality scores: round_scores[round] = comma-separated scores
declare -a round_avgs

for round in $(seq 1 "$ROUNDS"); do
    echo "--- Round $round/$ROUNDS ---"
    round_scores=()

    for i in $(seq 0 $((NUM_TASKS - 1))); do
        task_id=$(uuidgen 2>/dev/null || python3 -c "import uuid; print(uuid.uuid4())")
        prompt=$(jq -r ".tasks[$i].prompt" "$TASKS_FILE")
        category=$(jq -r ".tasks[$i].category" "$TASKS_FILE")

        # Submit task via WebSocket
        submit_msg=$(jq -n \
            --arg text "$prompt" \
            '{"type": "submitTask", "description": $text}')

        echo "  Task $((i+1))/$NUM_TASKS [$category]: submitting..."

        # Send submit message and collect RoutingOutcome events
        quality=""
        timeout_secs=$((WAIT_BETWEEN_TASKS + 60))

        # Use websocat to send and receive. Listen for RoutingOutcome with a timeout.
        quality=$(echo "$submit_msg" | timeout "$timeout_secs" websocat -n1 "$GATEWAY" 2>/dev/null | \
            timeout "$WAIT_BETWEEN_TASKS" head -20 | \
            jq -r 'select(.type == "routingOutcome") | .qualityScore // empty' 2>/dev/null | head -1) || true

        if [[ -z "$quality" ]]; then
            echo "    -> timeout/no quality score, using 0.0"
            quality="0.0"
        else
            echo "    -> quality: $quality"
        fi

        round_scores+=("$quality")
        sleep 2
    done

    # Compute round average
    sum=0
    count=0
    for score in "${round_scores[@]}"; do
        sum=$(echo "$sum + $score" | bc -l)
        count=$((count + 1))
    done
    avg=$(echo "scale=4; $sum / $count" | bc -l)
    round_avgs+=("$avg")
    echo "  Round $round avg quality: $avg"
    echo ""

    if [[ $round -lt $ROUNDS ]]; then
        echo "  Waiting ${WAIT_BETWEEN_ROUNDS}s for learning to propagate..."
        sleep "$WAIT_BETWEEN_ROUNDS"
    fi
done

# Compute overall average and slope (linear regression)
total_sum=0
for avg in "${round_avgs[@]}"; do
    total_sum=$(echo "$total_sum + $avg" | bc -l)
done
overall_avg=$(echo "scale=4; $total_sum / $ROUNDS" | bc -l)

# Simple slope: (last - first) / (n - 1)
if [[ $ROUNDS -gt 1 ]]; then
    first="${round_avgs[0]}"
    last="${round_avgs[$((ROUNDS-1))]}"
    slope=$(echo "scale=4; ($last - $first) / ($ROUNDS - 1)" | bc -l)
else
    slope="0.0"
fi

# Verdict: PASS if avg > 0.4 AND (slope >= 0 OR avg > 0.6)
if (( $(echo "$overall_avg > 0.4" | bc -l) )) && \
   (( $(echo "$slope >= 0" | bc -l) || $(echo "$overall_avg > 0.6" | bc -l) )); then
    verdict="PASS"
else
    verdict="FAIL"
fi

# Build JSON report
rounds_json="["
for r in $(seq 0 $((ROUNDS-1))); do
    [[ $r -gt 0 ]] && rounds_json+=","
    rounds_json+="{\"round\":$((r+1)),\"avgQuality\":${round_avgs[$r]}}"
done
rounds_json+="]"

timestamp=$(date -u +"%Y-%m-%dT%H:%M:%S+00:00")

jq -n \
    --arg gw "$GATEWAY" \
    --arg ts "$timestamp" \
    --arg verdict "$verdict" \
    --argjson avg "$overall_avg" \
    --argjson slope "$slope" \
    --argjson rounds "$rounds_json" \
    '{
        gateway: $gw,
        timestamp: $ts,
        verdict: $verdict,
        avgQuality: $avg,
        slope: $slope,
        rounds: $rounds
    }' > "$REPORT"

echo "=== Benchmark Complete ==="
echo "Verdict:  $verdict"
echo "Avg Quality: $overall_avg"
echo "Slope:    $slope"
echo "Report:   $REPORT"
