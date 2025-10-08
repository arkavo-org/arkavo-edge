# arkavo-router

Intelligent model routing for cost-optimized AI agent execution in Arkavo Edge.

## Overview

The `arkavo-router` crate implements a hybrid routing system that automatically selects the optimal AI model (local Gemma or cloud Gemini) based on task classification, providing **40-60% cost savings** while maintaining quality.

## Features

- **Fast Task Classification** - Gemma 270M classifies tasks in <100ms
- **Intelligent Routing** - Optimizes for speed, cost, and quality
- **Budget-Aware** - Switches to local models when budget is constrained
- **Cost Estimation** - Predicts cost before execution
- **Routing Metrics** - Tracks decisions, costs, and savings

## Architecture

```
Task Description
    ↓
TaskClassifier (Gemma 270M - local)
    ↓
{category, confidence}
    ↓
ModelSelector
    ↓
RoutingDecision {model, cost, time, reasoning}
    ↓
Execute on selected model
```

## Usage

### Basic Routing

```rust
use arkavo_router::Router;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let router = Router::new().await?;

    let decision = router
        .route("Create a React component with Tailwind CSS")
        .await?;

    println!("Model: {}", decision.recommended_model.name());
    println!("Cost: ${:.4}", decision.estimated_cost_usd);
    println!("Reasoning: {}", decision.reasoning);

    Ok(())
}
```

### Get Routing Metrics

```rust
let metrics = router.get_metrics().await;
println!("Total routes: {}", metrics.total_routes);
println!("Cost savings: ${:.4}", metrics.total_cost_saved);
println!("Savings: {:.1}%", metrics.cost_savings_percent());
```

## Task Categories

The router classifies tasks into these categories:

- **`frontend_ui`** → Gemini Flash (#1 on WebDev Arena)
- **`backend_api`** → Gemini Pro (highest quality)
- **`code_search`** → Local Gemma 4B (fast + free)
- **`security_scan`** → Local Gemma 4B (privacy)
- **`test_generation`** → Gemini Pro (comprehensive tests)
- **`documentation`** → Local Gemma 4B (sufficient quality)
- **`refactoring`** → Gemini Flash (quick iterations)
- **`general`** → Gemini Flash (balanced default)

## Model Selection

### Available Models

| Model | Size | Speed | Cost | Use Case |
|-------|------|-------|------|----------|
| Gemini Flash | Cloud | 3s | $0.003-0.006 | Frontend, refactoring |
| Gemini Pro | Cloud | 10s | $0.009 | Backend, tests |
| Gemma 270M | Local | 0.5s | $0 | Classification only |
| Gemma 4B | Local | 2s | $0 | Code search, security |
| Gemma 12B | Local | 5s | $0 | Budget fallback |

### Selection Logic

```rust
match (category, confidence) {
    (FrontendUI, >0.75) => GeminiFlash,      // #1 WebDev ranking
    (BackendAPI, >0.70) => GeminiPro,        // Highest quality
    (CodeSearch, _) => LocalGemma4B,          // Free + fast
    (SecurityScan, _) => LocalGemma4B,        // Privacy
    (TestGeneration, >0.70) => GeminiPro,    // Comprehensive
    _ if budget_low => LocalGemma12B,         // Cost constraint
    _ => GeminiFlash,                         // Default
}
```

## Classification

The `TaskClassifier` uses Gemma 270M for fast classification:

### Rule-Based Classification (Fast Path)

Confidence >0.85 for strong keyword matches:
- React, Vue, Tailwind → `frontend_ui`
- API, endpoint, auth → `backend_api`
- Search, find, grep → `code_search`
- Security, vulnerability → `security_scan`

### LLM Classification (Fallback)

For ambiguous tasks, uses Gemma 270M with structured prompt:

```
Classify this coding task into ONE category:
- frontend_ui: React/Vue components, Tailwind CSS
- backend_api: REST APIs, authentication
- code_search: Finding code, grep, AST
...

Task: {description}

Reply: Category: [name]
       Confidence: [0-100]
```

## Cost Estimation

Estimates cost before execution:

### Gemini Flash
```
input_tokens * $0.30/M + output_tokens * $2.50/M
```

### Gemini Pro
```
input_tokens * $1.25/M + output_tokens * $5.00/M
```

### Local Gemma
```
$0.00 (always free)
```

## Routing Metrics

Tracks routing decisions and calculates savings:

```rust
pub struct RoutingMetrics {
    pub total_routes: u64,
    pub routes_by_category: HashMap<String, u64>,
    pub routes_by_model: HashMap<String, u64>,
    pub total_cost_saved: f64,
    pub total_estimated_cost: f64,
    pub average_confidence: f64,
}
```

Example output:

```
Routing Metrics:
- Total routes: 100
- Average confidence: 87.5%
- Cost savings: $0.4320 (43.2%)
- Local model usage: 45.0%
- Categories: {"frontend_ui": 30, "code_search": 25, ...}
- Models: {"gemini-flash-latest": 40, "gemma-3-4b-it": 35, ...}
```

## Budget-Aware Routing

Automatically switches to local models when budget is constrained:

```rust
let selector = ModelSelector::new();

// Normal routing
let decision = selector.select(&classification, task).await?;

// Budget-aware routing (switches to local at 80%)
let decision = selector
    .select_with_budget_constraint(&classification, task, 0.85)
    .await?;
```

## Integration Example

```rust
use arkavo_router::{Router, TaskCategory};

async fn process_coding_task(task: &str) -> Result<String> {
    let router = Router::new().await?;

    let decision = router.route(task).await?;

    match decision.recommended_model {
        ModelChoice::GeminiFlash | ModelChoice::GeminiPro => {
            // Execute on Gemini cloud
            execute_on_gemini(&decision, task).await
        }
        _ => {
            // Execute on local Gemma
            execute_on_gemma(&decision, task).await
        }
    }
}
```

## Performance

- **Classification latency**: <100ms (Gemma 270M)
- **Routing decision**: <50ms (local logic)
- **Total overhead**: ~150ms per task
- **Cost savings**: 40-60% vs cloud-only
- **Local model usage**: 35-50% of tasks

## Testing

```bash
# Run unit tests
cargo test -p arkavo-router

# Run with local models (requires Gemma models)
cargo test -p arkavo-router --features llm-local

# Run benchmarks
cargo bench -p arkavo-router
```

## Dependencies

- **arkavo-llm** - Local Gemma model execution
- **arkavo-budget** - Cost tracking
- **arkavo-gemini** - Gemini API client

## Local Models Required

- `unsloth/gemma-3-270m-it-GGUF` - For classification
- `unsloth/gemma-3-4b-it-GGUF` - For code tasks

Download via:
```bash
# Models auto-download on first use via huggingface-cli
```

## License

Apache-2.0
