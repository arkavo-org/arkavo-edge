# A2A Protocol v2: Router Integration

## Overview

A2A Protocol v2 integrates the Router quality gate into the Agent-to-Agent communication protocol, enabling intelligent model selection, automatic quality validation, and cost optimization for A2A conversations.

## Changes from v1

### Architecture

**v1 (Current)**:
- ChatSessionManager directly uses `LlmClientAdapter`
- No routing - uses whatever model client is configured with
- No quality validation
- No automatic model escalation

**v2 (New)**:
- ChatSessionManager uses Router for intelligent model selection
- Quality gate with validator + judge checks
- Automatic model escalation on validation failures
- Tool execution via ToolRegistry

### Breaking Changes

1. **ChatSessionManager Constructor**
   - v1: `ChatSessionManager::new(llm_adapter: Arc<LlmClientAdapter>)`
   - v2: `ChatSessionManager::new()` (creates Router internally)

2. **Message Processing**
   - v1: Direct streaming via `llm_adapter.stream_chat()`
   - v2: Router-based with `router.route_with_quality_gate()`

3. **Tool Support**
   - v1: No native tool support
   - v2: Full MCP tool integration via ToolRegistry

## Implementation Guide

### ChatSessionManager Refactoring

**Before (v1)**:
```rust
pub struct ChatSessionManager {
    llm_adapter: Option<Arc<LlmClientAdapter>>,
    // ...
}

impl ChatSessionManager {
    pub fn new(llm_adapter: Option<Arc<LlmClientAdapter>>) -> Self {
        Self { llm_adapter, /* ... */ }
    }

    async fn handle_session(&mut self) {
        let response = self.llm_adapter.stream_chat(&messages).await?;
        // ...
    }
}
```

**After (v2)**:
```rust
use arkavo_router::Router;
use arkavo_mcp_tools::ToolRegistry;

pub struct ChatSessionManager {
    tool_registry: Option<ToolRegistry>,
    // ... (no llm_adapter)
}

impl ChatSessionManager {
    pub fn new(tool_registry: Option<ToolRegistry>) -> Self {
        Self { tool_registry, /* ... */ }
    }

    async fn handle_session(&mut self) {
        // Create Router per message for model selection
        let router = Router::new().await?;

        // Route with quality gate
        let response = router.route_with_quality_gate(
            &task_description,
            messages.clone(),
            self.tool_registry.as_ref(),
            3  // max retries
        ).await?;

        // Handle tool execution if needed
        if !response.tool_calls.is_empty() {
            // Execute tools and feed back to Router
        }

        // Send response
    }
}
```

### A2A Server Integration

**chat_send() Method**:
```rust
async fn chat_send(&self, session_id: String, message: UserMessage) -> RpcResult<()> {
    // Get or create session
    let session = self.sessions.get_or_create(&session_id).await?;

    // Add user message
    session.add_message(Message::user(&message.content)).await?;

    // Process with Router (internally uses quality gate)
    session.process_with_router().await?;

    Ok(())
}
```

**chat_subscribe() Method**:
```rust
async fn chat_subscribe(
    &self,
    session_id: String,
    sink: SubscriptionSink,
) -> SubscriptionResult {
    // Get or create session with Router support
    let mut session = self.sessions.get_or_create(&session_id).await?;

    // Stream responses with Router quality gate
    while let Some(response) = session.next_response().await {
        sink.send(&response).await?;
    }

    Ok(())
}
```

## Migration Path

Since A2A Protocol is not yet released, no migration path is needed. New implementations should use v2 directly.

## Benefits

1. **Intelligent Model Selection**: Router automatically selects the best model for each task
2. **Cost Optimization**: Starts with cheaper models (270M), escalates only when needed
3. **Quality Assurance**: Validator catches common errors (<1ms), Judge catches semantic issues (~500ms)
4. **Tool Support**: Full MCP tool integration out of the box
5. **Offline Support**: Gracefully falls back to local models when no API key available

## Feature Flags

- `llama-cpp`: Required for Judge validation (LLM-based quality checking)
- Without `llama-cpp`: Only Validator runs (fast syntax/type checking)

## Performance Characteristics

- **Routing Decision**: ~50-100ms (Gemma 270M classification)
- **Fast Validation**: <1ms per response
- **Judge Validation**: ~500ms per response (optional, llama-cpp feature)
- **Model Escalation**: Only on validation failures

## Testing

```bash
# Test ChatSessionManager with Router
cargo test -p arkavo-protocol --features llama-cpp

# Test without Judge (validator only)
cargo test -p arkavo-protocol
```

## Status

- ✅ Router quality gate implemented
- ✅ CLI commands integrated
- ⏳ ChatSessionManager refactoring (in progress)
- ⏳ A2A server methods (pending)
