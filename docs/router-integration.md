# Router Integration in Cognitive Engine

## Overview

The cognitive engine uses the `arkavo-router` for intelligent model selection and request routing. The router analyzes prompts and selects the most appropriate model based on complexity, cost, and quality requirements.

## Architecture

The router is integrated at multiple levels in the cognitive engine:

- **Planning Phase**: Uses router to select planning model (typically Gemini Flash/Pro)
- **Execution Phase**: Uses router with quality gate for step execution
- **Adjustment Phase**: Uses router for generating adjusted plans after failures

## Initialization

The cognitive engine is initialized with router and tool registry instances:

```rust
use arkavo_router::Router;
use arkavo_mcp_tools::ToolRegistry;

pub struct CognitiveEngine {
    router: Arc<Router>,
    tool_registry: Arc<ToolRegistry>,
    // ... other fields
}

impl CognitiveEngine {
    pub fn new(
        router: Arc<Router>,
        tool_registry: Arc<ToolRegistry>,
        // ... other parameters
    ) -> Self {
        Self {
            router,
            tool_registry,
            // ... initialize fields
        }
    }
}
```

**File Reference:** `crates/arkavo-orchestrator/src/cognitive_engine_core.rs:66-82`

## Router Usage Patterns

### Basic Routing

For simple model selection without tool calling:

```rust
let decision = self
    .router
    .route(&planning_prompt)
    .await
    .map_err(|e| Error::Other(anyhow::anyhow!("Routing failed: {e}")))?;

info!(
    model = ?decision.recommended_model,
    estimated_cost = decision.estimated_cost_usd,
    "Planning with selected model"
);
```

The router returns a `RoutingDecision` containing:
- `recommended_model`: The model to use (e.g., GeminiFlash, GeminiPro, LocalGemma4B)
- `estimated_cost_usd`: Cost estimate for the request
- `reasoning`: Why this model was selected

**File Reference:** `crates/arkavo-orchestrator/src/cognitive_engine_planning.rs:56-68`

### Quality Gate with Tool Registry

For execution steps that require tool calling with retry logic:

```rust
let response = self
    .router
    .route_with_quality_gate(
        command,                      // Short prompt for routing decision
        messages,                     // Full conversation messages
        Some(&self.tool_registry),   // Tools available for this request
        3                            // Maximum retry attempts
    )
    .await
    .map_err(|e| Error::Other(anyhow::anyhow!("Command execution failed: {e}")))?;

info!(
    tool_calls = response.tool_calls.len(),
    "Command executed with {} tool calls",
    response.tool_calls.len()
);
```

The quality gate provides:
- **Progressive tool disclosure**: Tools are only shown to models that need them
- **Automatic retries**: Up to N attempts with exponential backoff
- **Tool execution**: Handles tool calls and integrates results
- **Response validation**: Ensures response meets quality criteria

**File Reference:** `crates/arkavo-orchestrator/src/cognitive_engine_core.rs:446-458`

## Progressive Tool Disclosure

The router implements progressive tool disclosure to optimize token usage:

- **Gemini Flash/Pro**: Full MCP tool registry available
- **Local models**: Limited or no tools (context constraints)
- **Fallback**: Degraded service without tools if needed

This ensures that expensive API calls only include tools when the model can effectively use them.

## Model Selection Strategy

### Planning Phase

Planning uses high-quality models for strategic decisions:

```rust
match decision.recommended_model {
    arkavo_router::ModelChoice::LocalGemma4B
    | arkavo_router::ModelChoice::LocalGemma12B => {
        return Err(Error::Other(anyhow::anyhow!(
            "Local models not yet supported for planning. Set GEMINI_API_KEY for remote planning."
        )));
    }
    _ => {
        if let Some(gemini) = self.router.get_planning_provider() {
            Arc::new(gemini)
        } else {
            return Err(Error::Other(anyhow::anyhow!(
                "Planning model not available. Set GEMINI_API_KEY for remote planning."
            )));
        }
    }
}
```

**Current Strategy:**
- ✅ Remote planning: Gemini Flash or Pro
- ❌ Local planning: Not yet implemented (future: Gemma 4B/12B)

**File Reference:** `crates/arkavo-orchestrator/src/cognitive_engine_planning.rs:70-86`

### Execution Phase

Execution uses router's quality gate with automatic model selection based on task complexity:

```rust
let response = self
    .router
    .route_with_quality_gate(command, messages, Some(&self.tool_registry), 3)
    .await?;
```

The router analyzes:
- Command complexity
- Available budget
- Required capabilities (tools, context length)
- Cost vs quality tradeoff

**File Reference:** `crates/arkavo-orchestrator/src/cognitive_engine_core.rs:446-450`

### Adjustment Phase

Similar to planning, adjustments use high-quality models for strategic correction:

```rust
let decision = self
    .router
    .route(&adjustment_prompt)
    .await?;

let provider: Arc<dyn Provider> = if let Some(gemini) = self.router.get_planning_provider() {
    Arc::new(gemini)
} else {
    return Err(Error::Other(anyhow::anyhow!(
        "Adjustment requires Gemini. Set GEMINI_API_KEY."
    )));
};
```

**File Reference:** `crates/arkavo-orchestrator/src/cognitive_engine_planning.rs:228-248`

## Budget Tracking Integration

Router decisions include cost estimates that integrate with budget tracking:

```rust
let estimated_input_tokens = planning_prompt.len() as u32 / 4;
let estimated_output_tokens = response.len() as u32 / 4;

let usage = TokenUsage::new(estimated_input_tokens, estimated_output_tokens);
let cost = TokenCost::from_dollars(decision.estimated_cost_usd);

self.budget_tracker
    .record_spending(
        "github-orchestrator".to_string(),
        "gemini".to_string(),
        model_name.to_string(),
        usage,
        cost,
    )
    .await?;
```

**File Reference:** `crates/arkavo-orchestrator/src/cognitive_engine_planning.rs:101-126`

## Error Handling

Router errors are wrapped with context for better debugging:

```rust
.map_err(|e| Error::Other(anyhow::anyhow!("Routing failed: {e}")))?
.map_err(|e| Error::Other(anyhow::anyhow!("Command execution failed: {e}")))?
.map_err(|e| Error::Other(anyhow::anyhow!("Planning LLM call failed: {e}")))?
```

This provides clear error messages that include:
- Which phase failed (routing, execution, planning)
- Original error details
- Context for debugging

## Configuration

Router configuration is handled externally and passed to the cognitive engine:

```rust
// In orchestrator initialization:
let router = Arc::new(Router::new(/* config */));
let tool_registry = Arc::new(ToolRegistry::new(/* config */));

let engine = CognitiveEngine::new(
    budget_tracker,
    event_writer,
    github_ops,
    router,
    tool_registry,
    session_id,
);
```

**Environment Variables:**
- `GEMINI_API_KEY`: Required for remote planning/adjustment
- `GEMINI_MODEL`: Optional model override (defaults to router selection)
- Router may respect additional model-specific environment variables

## Testing

Router integration can be tested with mock implementations:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;

    mock! {
        Router {}
        impl Router {
            async fn route(&self, prompt: &str) -> Result<RoutingDecision>;
            async fn route_with_quality_gate(
                &self,
                prompt: &str,
                messages: Vec<LlmMessage>,
                tools: Option<&ToolRegistry>,
                max_attempts: u32,
            ) -> Result<ToolResponse>;
        }
    }

    // Test router integration with mock
}
```

## Performance Considerations

### Token Optimization

The router helps optimize token usage through:
- **Model selection**: Cheaper models for simple tasks
- **Progressive disclosure**: Only include tools when needed
- **Context management**: Right-size context window for task

### Cost Management

Router provides cost estimates before execution:
```rust
info!(
    estimated_cost = decision.estimated_cost_usd,
    "Planning with estimated cost ${:.6}",
    decision.estimated_cost_usd
);
```

This allows budget checks before committing to expensive operations.

### Quality Guarantees

Quality gate ensures acceptable results:
- Multiple retry attempts for transient failures
- Model fallback for capability issues
- Tool execution validation

## Future Improvements

### Local Model Support

Enable local models for planning/adjustment:
```rust
match decision.recommended_model {
    arkavo_router::ModelChoice::LocalGemma4B => {
        let provider = self.router.get_local_provider()?;
        // Use local Gemma 4B for planning
    }
    // ...
}
```

### Streaming Support

Add streaming for long-running executions:
```rust
let stream = self
    .router
    .route_with_quality_gate_streaming(command, messages, tools, 3)
    .await?;

while let Some(chunk) = stream.next().await {
    // Process streaming response
    self.post_progress(&assignment, &chunk).await?;
}
```

### Advanced Routing

Implement more sophisticated routing strategies:
- Cost-aware routing with budget constraints
- Quality-of-service tiers (fast, balanced, accurate)
- Multi-model ensemble for critical decisions
- Caching for repeated prompts

## Related Documentation

- **Router Implementation**: `crates/arkavo-router/README.md`
- **Tool Registry**: `crates/arkavo-mcp-tools/README.md`
- **GitHub Orchestrator Architecture**: `docs/GITHUB_ORCHESTRATOR_INFRASTRUCTURE.md`
- **Budget Tracking**: `crates/arkavo-budget/README.md`
