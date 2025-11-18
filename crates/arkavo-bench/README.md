# arkavo-bench

Benchmarking utilities for Arkavo Edge performance evaluation with comprehensive SWE-bench support.

## Features

- **Multiple SWE-bench Variants**: Support for Lite (534), Verified (500), Full (2294), and Multimodal (500) instances
- **Arkavo-Assisted Mode**: Production-ready code generation with intelligent context analysis and quality gates
- **Parallel Execution**: Run multiple benchmark instances concurrently for faster evaluation
- **HuggingFace Integration**: Direct loading from official SWE-bench datasets
- **Comprehensive Metrics**: Track resolution rates, wall time, API calls, tokens, and costs with ResponseJudge integration
- **MCP Tool Integration**: Seamless integration with Arkavo's MCP tool ecosystem
- **Solution Validation**: Multi-framework test execution (pytest, cargo test, jest)

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

### Arkavo-Assisted Mode (Production)

Use the production-ready Arkavo-assisted solver with intelligent context analysis and quality gates:

```rust
use arkavo_bench::{ArkavoMode, SweBenchInstance};
use arkavo_router::Router;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Initialize router with quality gate
    let router = Arc::new(Router::new().await.unwrap());
    let arkavo = ArkavoMode::new(router).await.unwrap();

    // Load SWE-bench instance
    let instance = SweBenchInstance::new(
        "django__django-12345".to_string(),
        "https://github.com/django/django".to_string(),
        "abc123def".to_string(),
        "Fix authentication bug...".to_string(),
        Some("Check password validation".to_string()),
        "test.patch".to_string(),
    );

    // Run with Arkavo assistance
    let workspace = PathBuf::from("/tmp/workspace");
    let metrics = arkavo.run_instance(&instance, &workspace).await.unwrap();

    println!("Resolved: {}", metrics.resolved);
    println!("Quality Gate Passed: {:?}", metrics.quality_gate_passed);
    println!("Retries: {}", metrics.quality_retries);
    println!("Issue Type: {:?}", metrics.issue_type);
}
```

### Comparative Benchmarking

Compare raw LLM vs Arkavo-assisted approaches:

```rust
use arkavo_bench::ComparativeRunner;

let runner = ComparativeRunner::new(router).await.unwrap();
let comparison = runner.run_comparison(&instance, &workspace).await.unwrap();

println!("Improvement: {:.1}%", comparison.improvement_percentage());
println!("Speedup: {:.2}x", comparison.speedup_factor());
println!("Cost difference: ${:.2}", comparison.cost_difference_usd());
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

### Basic Metrics
- `instance_id`: SWE-bench instance identifier
- `resolved`: Whether the instance was successfully resolved
- `wall_time_ms`: Total execution time in milliseconds
- `api_calls`: Number of LLM API calls made
- `total_tokens`: Total tokens used (input + output)
- `estimated_cost_usd`: Estimated cost in USD
- `error_message`: Error details if execution failed

### Arkavo-Assisted Metrics (with ResponseJudge)
- `quality_gate_passed`: Whether ResponseJudge approved the solution
- `quality_retries`: Number of retry attempts with model escalation
- `issue_type`: Type of issue detected (none, hallucinated_tool, invalid_params, refusal, off_topic)
- `judge_reason`: Explanation from ResponseJudge

### Summary Statistics
- `quality_gate_pass_rate`: Percentage of solutions passing quality gates
- `avg_quality_retries`: Average number of retries across instances
- `issue_type_breakdown`: Distribution of detected issues

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
