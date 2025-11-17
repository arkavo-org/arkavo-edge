# arkavo-bench

Benchmarking utilities for Arkavo Edge performance evaluation with comprehensive SWE-bench support.

## Features

- **Multiple SWE-bench Variants**: Support for Lite (534), Verified (500), Full (2294), and Multimodal (500) instances
- **Parallel Execution**: Run multiple benchmark instances concurrently for faster evaluation
- **HuggingFace Integration**: Direct loading from official SWE-bench datasets
- **Comprehensive Metrics**: Track resolution rates, wall time, API calls, tokens, and costs
- **MCP Tool Integration**: Seamless integration with Arkavo's MCP tool ecosystem

## Usage

### Load Benchmark Instances

```rust
use arkavo_bench::SweBenchTool;
use arkavo_mcp::Tool;
use serde_json::json;

#[tokio::main]
async fn main() {
    let tool = SweBenchTool::new();

    let params = json!({
        "action": "load",
        "subset": "lite",  // Options: lite, verified, full, multimodal
        "limit": 10
    });

    let result = tool.execute(params).await.unwrap();
    println!("Loaded: {:?}", result);
}
```

### Run Benchmarks with Parallel Execution

```rust
let params = json!({
    "action": "run",
    "subset": "lite",
    "limit": 50,
    "parallel": 4,  // Run 4 instances concurrently
    "metrics_file": "/tmp/results.json"
});

let result = tool.execute(params).await.unwrap();
```

### Evaluate Solutions

```rust
let params = json!({
    "action": "evaluate",
    "subset": "lite",
    "instance_id": "django__django-12345",
    "solution": "diff --git a/... (git patch)"
});

let result = tool.execute(params).await.unwrap();
let resolved = result["resolved"].as_bool().unwrap();
```

### Generate Summary Reports

```rust
let params = json!({
    "action": "summary",
    "metrics_file": "/tmp/results.json"
});

let summary = tool.execute(params).await.unwrap();
// Returns: total_instances, resolved_count, resolved_percentage,
//          avg_wall_time_ms, total_cost_usd, etc.
```

## SWE-bench Datasets

| Dataset | Instances | Description |
|---------|-----------|-------------|
| **Lite** | 534 | Curated subset for faster evaluation |
| **Verified** | 500 | Verified solvable instances |
| **Full** | 2,294 | Complete benchmark dataset |
| **Multimodal** | 500 | Multimodal tasks (test split) |

## Examples

See `examples/swe-bench-baseline.rs` for a basic test runner and `examples/swe-bench-gemini.rs` for a complete integration with Gemini 2.5 models.

### Run Baseline Test

```bash
cargo run --example swe-bench-baseline
```

### Run Gemini Benchmark

```bash
GEMINI_API_KEY=your_key cargo run --example swe-bench-gemini
```

## Metrics

Each benchmark run produces detailed metrics:

- `instance_id`: SWE-bench instance identifier
- `resolved`: Whether the instance was successfully resolved
- `wall_time_ms`: Total execution time in milliseconds
- `api_calls`: Number of LLM API calls made
- `total_tokens`: Total tokens used (input + output)
- `estimated_cost_usd`: Estimated cost in USD
- `error_message`: Error details if execution failed

## Integration with Gemini

The benchmark tool is designed to work seamlessly with Arkavo's Gemini integration:

```rust
use arkavo_gemini::GeminiClient;
use arkavo_bench::SweBenchTool;

// 1. Load benchmark instance
// 2. Generate solution with Gemini
// 3. Evaluate solution with benchmark tool
// 4. Track metrics and costs
```

## Architecture

- **SweBenchTool**: MCP tool implementation for SWE-bench operations
- **BenchMetrics**: Per-instance execution metrics
- **BenchSummary**: Aggregated statistics across multiple runs
- **WorkspaceTool**: Sandboxed execution environment integration

## Future Enhancements

- [ ] HumanEval and MBPP benchmarks
- [ ] CodeContests integration
- [ ] Real-time streaming metrics
- [ ] Comparative analysis dashboards
- [ ] Automated regression testing
- [ ] Public leaderboard integration
