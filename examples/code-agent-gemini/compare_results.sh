#!/bin/bash

# Compare Gemini vs Claude (or other models) on coding benchmarks
# Runs identical tasks with both models and generates comparison report

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Configuration
RESULTS_DIR="$(cd "$(dirname "$0")" && pwd)/results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
COMPARISON_DIR="$RESULTS_DIR/comparisons"
GEMINI_PORT=8346
CLAUDE_PORT=8345

mkdir -p "$COMPARISON_DIR"

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

check_agents() {
    local model1="$1"
    local model2="$2"

    print_status "Checking agent availability..."

    if [[ "$model1" == gemini* ]]; then
        if ! curl -s "http://localhost:$GEMINI_PORT/health" > /dev/null 2>&1; then
            print_error "Gemini agent not running on port $GEMINI_PORT"
            echo "Start it with: ./launch_agent.sh"
            exit 1
        fi
        print_status "Gemini agent: OK"
    fi

    if [[ "$model2" == claude* ]]; then
        if ! curl -s "http://localhost:$CLAUDE_PORT/health" > /dev/null 2>&1; then
            print_error "Claude agent not running on port $CLAUDE_PORT"
            echo "Start it with: cd ../claude-code-agent && ./launch_agent.sh"
            exit 1
        fi
        print_status "Claude agent: OK"
    fi
}

run_task_with_model() {
    local model="$1"
    local task_name="$2"
    local task_prompt="$3"
    local output_file="$4"

    # Determine port and feature based on model
    local port=$GEMINI_PORT
    local features="gemini"

    if [[ "$model" == claude* ]]; then
        port=$CLAUDE_PORT
        features="claude"
    fi

    print_status "Running '$task_name' with $model..."

    local start_time=$(date +%s%N)

    # Send task to appropriate agent
    local response=$(curl -s -w "\n%{time_total}" -X POST "http://localhost:$port/v1/agent/task" \
        -H "Content-Type: application/json" \
        -d "{
            \"task\": \"$task_prompt\",
            \"model\": \"$model\"
        }" 2>&1)

    local end_time=$(date +%s%N)
    local duration_ms=$(( ($end_time - $start_time) / 1000000 ))

    # Extract curl timing (last line)
    local curl_time=$(echo "$response" | tail -1)
    local response_body=$(echo "$response" | head -n -1)

    # Parse response and add metrics
    echo "$response_body" | jq --arg model "$model" \
        --arg dur "$duration_ms" \
        --arg task "$task_name" \
        '. + {
            model: $model,
            task_name: $task,
            duration_ms: $dur|tonumber,
            timestamp: now
        }' > "$output_file" 2>/dev/null || {
        # Fallback if response is not JSON
        jq -n --arg model "$model" \
            --arg dur "$duration_ms" \
            --arg task "$task_name" \
            --arg body "$response_body" \
            '{
                model: $model,
                task_name: $task,
                duration_ms: $dur|tonumber,
                response: $body,
                timestamp: now
            }' > "$output_file"
    }

    print_status "$model completed in ${duration_ms}ms"
}

run_comparison() {
    local model1="${1:-gemini-3-pro-preview}"
    local model2="${2:-claude-3-7-sonnet}"
    local iterations="${3:-3}"

    print_header "Comparing $model1 vs $model2"

    check_agents "$model1" "$model2"

    local comp_dir="$COMPARISON_DIR/${model1}_vs_${model2}_${TIMESTAMP}"
    mkdir -p "$comp_dir"

    # Define test tasks
    local tasks=(
        "frontend_component:Create a responsive React card component with Tailwind CSS showing user profile"
        "rest_api:Write a Node.js Express REST API for user authentication with JWT"
        "test_generation:Generate Jest tests for a TodoList component with CRUD operations"
        "bug_fix:Fix this code - function add(a,b){return a+b} should handle string concatenation"
        "code_review:Review and improve this code - const x = [1,2,3]; for(var i=0;i<x.length;i++){console.log(x[i])}"
    )

    # Run each task with both models
    for task_def in "${tasks[@]}"; do
        IFS=':' read -r task_name task_prompt <<< "$task_def"

        print_header "Task: $task_name"

        for iter in $(seq 1 "$iterations"); do
            print_status "Iteration $iter/$iterations"

            # Model 1
            local file1="$comp_dir/${task_name}_${model1}_iter${iter}.json"
            run_task_with_model "$model1" "$task_name" "$task_prompt" "$file1"

            # Model 2
            local file2="$comp_dir/${task_name}_${model2}_iter${iter}.json"
            run_task_with_model "$model2" "$task_name" "$task_prompt" "$file2"

            # Brief pause between iterations
            sleep 1
        done
    done

    print_status "All tasks completed"
    print_status "Generating comparison report..."
    generate_comparison_report "$comp_dir" "$model1" "$model2"
}

generate_comparison_report() {
    local comp_dir="$1"
    local model1="$2"
    local model2="$3"

    local report_file="$comp_dir/comparison_report.md"

    cat > "$report_file" << EOF
# Coding Agent Comparison Report

**Model 1**: $model1
**Model 2**: $model2
**Generated**: $(date)

## Executive Summary

EOF

    # Calculate aggregate metrics
    local model1_avg_time=$(jq -s '[.[] | select(.model == "'$model1'") | .duration_ms] | add / length | floor' "$comp_dir"/*.json)
    local model2_avg_time=$(jq -s '[.[] | select(.model == "'$model2'") | .duration_ms] | add / length | floor' "$comp_dir"/*.json)

    local faster_model="$model1"
    local speedup="1.0"
    if [ "$model1_avg_time" -lt "$model2_avg_time" ]; then
        speedup=$(echo "scale=2; $model2_avg_time / $model1_avg_time" | bc)
        faster_model="$model1"
    else
        speedup=$(echo "scale=2; $model1_avg_time / $model2_avg_time" | bc)
        faster_model="$model2"
    fi

    cat >> "$report_file" << EOF
| Metric | $model1 | $model2 | Winner |
|--------|---------|---------|--------|
| **Avg Completion Time** | ${model1_avg_time}ms | ${model2_avg_time}ms | **$faster_model** (${speedup}x) |

**Fastest Model**: $faster_model is ${speedup}x faster on average

## Task-by-Task Breakdown

EOF

    # Analyze each task
    for task_name in frontend_component rest_api test_generation bug_fix code_review; do
        if ls "$comp_dir"/${task_name}_*.json 1> /dev/null 2>&1; then
            local m1_times=$(jq -s '[.[] | select(.model == "'$model1'" and .task_name == "'$task_name'") | .duration_ms]' "$comp_dir"/*.json)
            local m2_times=$(jq -s '[.[] | select(.model == "'$model2'" and .task_name == "'$task_name'") | .duration_ms]' "$comp_dir"/*.json)

            local m1_avg=$(echo "$m1_times" | jq 'add / length | floor')
            local m2_avg=$(echo "$m2_times" | jq 'add / length | floor')
            local m1_min=$(echo "$m1_times" | jq 'min')
            local m2_min=$(echo "$m2_times" | jq 'min')

            cat >> "$report_file" << EOF
### $task_name

| Model | Avg Time | Min Time | Max Time |
|-------|----------|----------|----------|
| $model1 | ${m1_avg}ms | ${m1_min}ms | $(echo "$m1_times" | jq 'max')ms |
| $model2 | ${m2_avg}ms | ${m2_min}ms | $(echo "$m2_times" | jq 'max')ms |

EOF
        fi
    done

    # Cost analysis (if pricing info available)
    cat >> "$report_file" << EOF

## Cost Analysis

Based on published pricing:

| Model | Input Cost | Output Cost | Notes |
|-------|-----------|-------------|-------|
| gemini-3-pro-preview | \$? / 1M tokens | \$? / 1M tokens | High quality |
| gemini-2.5-flash | \$0.30 / 1M tokens | \$2.50 / 1M tokens | Fast iteration |
| claude-3-7-sonnet | \$? / 1M tokens | \$? / 1M tokens | Best SWE-bench |

*Note: Actual token counts would need to be captured from API responses*

## Recommendations

EOF

    # Generate recommendations
    if [ "$faster_model" = "$model1" ]; then
        echo "- **Speed**: Use $model1 for faster iteration (${speedup}x faster)" >> "$report_file"
    else
        echo "- **Speed**: Use $model2 for faster iteration (${speedup}x faster)" >> "$report_file"
    fi

    cat >> "$report_file" << EOF
- **Frontend Tasks**: Gemini 2.5 Pro excels at WebDev (ranked #1 on WebDev Arena)
- **General Coding**: Claude 3.7 Sonnet has higher SWE-bench score (70-72% vs 63-67%)
- **Budget**: Gemini Flash offers best cost/performance ratio
- **Quality**: Both models are production-ready; choose based on task type

## Raw Data

All individual task results available in:
\`$comp_dir\`

EOF

    print_status "Report generated: $report_file"
    echo ""
    cat "$report_file"
}

analyze_existing_results() {
    print_header "Analyzing Existing Results"

    if [ ! -d "$RESULTS_DIR" ] || [ -z "$(ls -A "$RESULTS_DIR" 2>/dev/null)" ]; then
        print_error "No results found in $RESULTS_DIR"
        echo "Run benchmarks first: ./run_benchmarks.sh all"
        exit 1
    fi

    print_status "Found results in $RESULTS_DIR"

    # Group by model
    local models=$(jq -r '.model // empty' "$RESULTS_DIR"/*.json 2>/dev/null | sort -u)

    if [ -z "$models" ]; then
        print_error "No model information in results"
        exit 1
    fi

    echo ""
    echo "Available models:"
    echo "$models"
    echo ""

    # Generate summary for each model
    for model in $models; do
        print_status "Model: $model"

        local avg_time=$(jq -s --arg m "$model" \
            '[.[] | select(.model == $m) | .duration_ms // .total_ms // 0] | add / length | floor' \
            "$RESULTS_DIR"/*.json)

        local count=$(jq -s --arg m "$model" \
            '[.[] | select(.model == $m)] | length' \
            "$RESULTS_DIR"/*.json)

        echo "  Tasks: $count"
        echo "  Avg Time: ${avg_time}ms"
        echo ""
    done
}

show_usage() {
    cat << EOF
Usage: $0 [command] [options]

Commands:
  run <model1> <model2> [iterations]  - Run comparison between two models
                                        Default: gemini-3-pro-preview vs claude-3-7-sonnet
                                        iterations: default 3

  analyze                              - Analyze existing results in results/

  report <comparison_dir>              - Generate report from existing comparison

Options:
  --output <file>                      - Output file for report (default: comparison_report.md)

Examples:
  # Compare Gemini Pro vs Claude Sonnet
  $0 run gemini-3-pro-preview claude-3-7-sonnet

  # Compare Gemini Flash vs Gemini Pro
  $0 run gemini-flash-latest gemini-3-pro-preview 5

  # Analyze all existing results
  $0 analyze

  # Generate report from specific comparison
  $0 report results/comparisons/gemini-3-pro-preview_vs_claude-3-7-sonnet_20250107_143022

EOF
}

# Main command handling
case "${1:-help}" in
    run)
        run_comparison "${2:-gemini-3-pro-preview}" "${3:-claude-3-7-sonnet}" "${4:-3}"
        ;;
    analyze)
        analyze_existing_results
        ;;
    report)
        if [ -z "$2" ]; then
            print_error "Comparison directory required"
            echo "Usage: $0 report <comparison_dir>"
            exit 1
        fi
        if [ ! -d "$2" ]; then
            print_error "Directory not found: $2"
            exit 1
        fi
        # Extract model names from directory
        local dir_name=$(basename "$2")
        local model1=$(echo "$dir_name" | cut -d'_' -f1)
        local model2=$(echo "$dir_name" | cut -d'_' -f3)
        generate_comparison_report "$2" "$model1" "$model2"
        ;;
    help|-h|--help)
        show_usage
        ;;
    *)
        print_error "Unknown command: $1"
        show_usage
        exit 1
        ;;
esac
