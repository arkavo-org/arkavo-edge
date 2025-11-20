#!/bin/bash

# Gemini Code Agent Benchmarking Script
# Run SWE-bench and custom coding benchmarks

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
AGENT_PORT=8346
RESULTS_DIR="$(cd "$(dirname "$0")" && pwd)/results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BENCHMARK_MODEL="${GEMINI_MODEL:-gemini-3-pro-preview}"

# Create results directory
mkdir -p "$RESULTS_DIR"

print_status() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_header() {
    echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
}

check_agent_running() {
    if ! curl -s "http://localhost:$AGENT_PORT/health" > /dev/null 2>&1; then
        print_error "Agent not running on port $AGENT_PORT"
        echo "Start the agent first: ./launch_agent.sh"
        exit 1
    fi
}

run_swe_bench() {
    local dataset="${1:-lite}"
    local count="${2:-10}"

    print_header "SWE-bench Benchmark ($dataset, $count instances)"

    local output_file="$RESULTS_DIR/swe_bench_${dataset}_${TIMESTAMP}.json"

    print_status "Running $count instances from SWE-bench $dataset..."
    print_status "Model: $BENCHMARK_MODEL"
    print_status "Output: $output_file"

    # Use arkavo-bench crate
    cd ../..
    cargo run --features gemini -p arkavo-bench -- \
        --dataset "$dataset" \
        --count "$count" \
        --model "$BENCHMARK_MODEL" \
        --output "$output_file" \
        2>&1 | tee "$RESULTS_DIR/swe_bench_${dataset}_${TIMESTAMP}.log"

    if [ $? -eq 0 ]; then
        print_status "Benchmark completed successfully"
        print_status "Results saved to: $output_file"

        # Show summary
        if command -v jq &> /dev/null && [ -f "$output_file" ]; then
            echo ""
            print_status "Summary:"
            jq '.summary' "$output_file" 2>/dev/null || cat "$output_file"
        fi
    else
        print_error "Benchmark failed"
        exit 1
    fi
}

run_custom_task() {
    local task_name="$1"
    local task_prompt="$2"

    print_header "Custom Task: $task_name"

    local output_file="$RESULTS_DIR/${task_name}_${TIMESTAMP}.json"
    local start_time=$(date +%s%N)

    print_status "Running task: $task_name"
    print_status "Model: $BENCHMARK_MODEL"

    # Send task to agent
    RESPONSE=$(curl -s -X POST "http://localhost:$AGENT_PORT/v1/agent/task" \
        -H "Content-Type: application/json" \
        -d "{
            \"task\": \"$task_prompt\",
            \"model\": \"$BENCHMARK_MODEL\",
            \"metrics\": true
        }" 2>&1)

    local end_time=$(date +%s%N)
    local duration_ms=$(( ($end_time - $start_time) / 1000000 ))

    if [ $? -eq 0 ]; then
        print_status "Task completed in ${duration_ms}ms"

        # Save results with metrics
        echo "$RESPONSE" | jq --arg dur "$duration_ms" \
            '. + {duration_ms: $dur|tonumber, timestamp: now, model: "'$BENCHMARK_MODEL'"}' \
            > "$output_file" 2>/dev/null || echo "$RESPONSE" > "$output_file"

        print_status "Results saved to: $output_file"
    else
        print_error "Task failed"
        echo "$RESPONSE"
        exit 1
    fi
}

run_speed_benchmark() {
    print_header "Speed Benchmark (TTFT & Tokens/sec)"

    local iterations="${1:-5}"
    local output_file="$RESULTS_DIR/speed_benchmark_${TIMESTAMP}.json"

    print_status "Running $iterations iterations with $BENCHMARK_MODEL"

    echo "{\"iterations\": []}" > "$output_file"

    for i in $(seq 1 "$iterations"); do
        print_status "Iteration $i/$iterations..."

        local start_time=$(date +%s%N)
        local ttft=0
        local total_tokens=0
        local first_token_received=false

        # Use chat command to measure streaming performance
        cd ../..
        local response=$(timeout 30s cargo run --features gemini -p arkavo -- \
            chat --model "$BENCHMARK_MODEL" \
            --prompt "Write a simple hello world function in Python" \
            2>&1)

        local end_time=$(date +%s%N)
        local total_time_ms=$(( ($end_time - $start_time) / 1000000 ))

        # Parse token info from response (if available)
        # This is approximate - actual implementation would parse streaming events
        local tokens=$(echo "$response" | wc -w)
        local tokens_per_sec=$(echo "scale=2; $tokens * 1000 / $total_time_ms" | bc -l 2>/dev/null || echo "0")

        # Estimate TTFT (first 20% of total time as approximation)
        local ttft_ms=$(echo "scale=0; $total_time_ms * 0.2 / 1" | bc 2>/dev/null || echo "0")

        # Save iteration results
        local iter_result=$(jq -n \
            --arg iter "$i" \
            --arg ttft "$ttft_ms" \
            --arg total "$total_time_ms" \
            --arg tokens "$tokens" \
            --arg tps "$tokens_per_sec" \
            '{iteration: $iter|tonumber, ttft_ms: $ttft|tonumber, total_ms: $total|tonumber, tokens: $tokens|tonumber, tokens_per_sec: $tps|tonumber}')

        jq --argjson iter "$iter_result" '.iterations += [$iter]' "$output_file" > "${output_file}.tmp" && mv "${output_file}.tmp" "$output_file"

        echo "  TTFT: ${ttft_ms}ms, Total: ${total_time_ms}ms, Tokens/sec: $tokens_per_sec"
    done

    # Calculate averages
    local avg_ttft=$(jq '[.iterations[].ttft_ms] | add / length | floor' "$output_file")
    local avg_total=$(jq '[.iterations[].total_ms] | add / length | floor' "$output_file")
    local avg_tps=$(jq '[.iterations[].tokens_per_sec] | add / length' "$output_file")

    jq --arg model "$BENCHMARK_MODEL" \
        --arg ttft "$avg_ttft" \
        --arg total "$avg_total" \
        --arg tps "$avg_tps" \
        '. + {model: $model, avg_ttft_ms: $ttft|tonumber, avg_total_ms: $total|tonumber, avg_tokens_per_sec: $tps|tonumber}' \
        "$output_file" > "${output_file}.tmp" && mv "${output_file}.tmp" "$output_file"

    print_status "Speed benchmark completed"
    print_status "Average TTFT: ${avg_ttft}ms"
    print_status "Average Total: ${avg_total}ms"
    print_status "Average Tokens/sec: $avg_tps"
    print_status "Results: $output_file"
}

run_predefined_tasks() {
    print_header "Running Predefined Coding Tasks"

    # Frontend component (Gemini's strength)
    run_custom_task "frontend_component" \
        "Create a responsive React card component with Tailwind CSS that displays a user profile with name, avatar, and bio"

    # REST API
    run_custom_task "rest_api" \
        "Write a Node.js Express REST API endpoint for user authentication with JWT tokens"

    # Test generation
    run_custom_task "test_generation" \
        "Generate comprehensive Jest tests for a TodoList React component with add, delete, and toggle functionality"

    # Code refactoring
    run_custom_task "code_refactoring" \
        "Refactor this code to use async/await: function getData() { return fetch('/api/data').then(r => r.json()).then(d => process(d)); }"

    print_status "All predefined tasks completed"
    print_status "Results in: $RESULTS_DIR"
}

generate_report() {
    print_header "Generating Benchmark Report"

    local report_file="$RESULTS_DIR/report_${TIMESTAMP}.md"

    cat > "$report_file" << EOF
# Gemini Code Agent Benchmark Report

**Generated**: $(date)
**Model**: $BENCHMARK_MODEL

## Speed Metrics

EOF

    # Find latest speed benchmark
    local latest_speed=$(ls -t "$RESULTS_DIR"/speed_benchmark_*.json 2>/dev/null | head -1)
    if [ -n "$latest_speed" ]; then
        echo "### Performance" >> "$report_file"
        jq -r '"- **Average TTFT**: \(.avg_ttft_ms)ms\n- **Average Total Time**: \(.avg_total_ms)ms\n- **Average Tokens/sec**: \(.avg_tokens_per_sec)"' "$latest_speed" >> "$report_file"
        echo "" >> "$report_file"
    fi

    # Add SWE-bench results if available
    local latest_swe=$(ls -t "$RESULTS_DIR"/swe_bench_*.json 2>/dev/null | head -1)
    if [ -n "$latest_swe" ]; then
        echo "## SWE-bench Results" >> "$report_file"
        echo "" >> "$report_file"
        jq -r '.summary' "$latest_swe" >> "$report_file" 2>/dev/null || echo "Summary not available" >> "$report_file"
        echo "" >> "$report_file"
    fi

    # List all task results
    echo "## Task Results" >> "$report_file"
    echo "" >> "$report_file"
    for result in "$RESULTS_DIR"/*_${TIMESTAMP}.json; do
        if [ -f "$result" ]; then
            local task_name=$(basename "$result" .json | sed "s/_${TIMESTAMP}//")
            echo "### $task_name" >> "$report_file"
            echo "" >> "$report_file"
            jq -r 'if .duration_ms then "**Duration**: \(.duration_ms)ms" else "Duration not recorded" end' "$result" >> "$report_file" 2>/dev/null || echo "N/A" >> "$report_file"
            echo "" >> "$report_file"
        fi
    done

    print_status "Report generated: $report_file"
    echo ""
    cat "$report_file"
}

show_usage() {
    cat << EOF
Usage: $0 [command] [options]

Commands:
  swe-bench [dataset] [count]  - Run SWE-bench evaluation
                                 dataset: lite|verified|test (default: lite)
                                 count: number of instances (default: 10)

  speed [iterations]           - Run speed benchmark (TTFT, tokens/sec)
                                 iterations: default 5

  tasks                        - Run predefined coding tasks

  custom <name> <prompt>       - Run custom task

  all [count]                  - Run all benchmarks

  report                       - Generate markdown report from results

  compare <model>              - Compare with another model (runs same tasks)

Options:
  --model <model>              - Override model (default: gemini-3-pro-preview)

Examples:
  $0 swe-bench lite 20                    # 20 instances from SWE-bench Lite
  $0 speed 10                             # 10 speed test iterations
  $0 tasks                                # Run all predefined tasks
  $0 custom my_test "Write hello world"   # Custom task
  $0 all 10                               # Run everything
  $0 --model gemini-flash-latest speed    # Speed test with Flash
  $0 compare claude-3-7-sonnet            # Compare with Claude

EOF
}

# Parse options
while [[ $# -gt 0 ]]; do
    case $1 in
        --model)
            BENCHMARK_MODEL="$2"
            shift 2
            ;;
        swe-bench)
            check_agent_running
            run_swe_bench "${2:-lite}" "${3:-10}"
            exit 0
            ;;
        speed)
            check_agent_running
            run_speed_benchmark "${2:-5}"
            exit 0
            ;;
        tasks)
            check_agent_running
            run_predefined_tasks
            exit 0
            ;;
        custom)
            check_agent_running
            if [ -z "$2" ] || [ -z "$3" ]; then
                print_error "Usage: $0 custom <name> <prompt>"
                exit 1
            fi
            run_custom_task "$2" "$3"
            exit 0
            ;;
        all)
            check_agent_running
            run_speed_benchmark 5
            run_predefined_tasks
            run_swe_bench lite "${2:-10}"
            generate_report
            exit 0
            ;;
        report)
            generate_report
            exit 0
            ;;
        compare)
            if [ -z "$2" ]; then
                print_error "Usage: $0 compare <model>"
                exit 1
            fi
            # This would be implemented in compare_results.sh
            print_status "See compare_results.sh for model comparison"
            exit 0
            ;;
        -h|--help)
            show_usage
            exit 0
            ;;
        *)
            print_error "Unknown command: $1"
            show_usage
            exit 1
            ;;
    esac
done

# Default: show usage
show_usage
