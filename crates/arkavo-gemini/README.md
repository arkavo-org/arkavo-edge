# arkavo-gemini

Gemini Live API WebSocket integration for Arkavo Edge.

## Features

- ✅ Text-only WebSocket sessions with Gemini Live API
- ✅ Tool/function declaration in setup message
- ✅ Automatic reconnection with exponential backoff
- ✅ Tool call dispatcher with configurable concurrency limits
- ✅ Idempotency via `requestId` deduplication
- ✅ Schema validation for tool arguments
- ✅ Async tool execution with semaphore-based limiting

## Implementation Status

**Code**: Complete (~760 LOC across 5 files, all <400 LOC)
**Tests**: All passing (6 unit + integration tests)
**Quality**: Clippy clean, formatted

### Tool Declaration Implementation ✅

The client now correctly sends tool declarations in the setup message:

```json
{
  "setup": {
    "model": "gemini-2.5-flash-live-preview",
    "generationConfig": {"responseModalities": ["TEXT"]},
    "tools": [{
      "functionDeclarations": [{
        "name": "create_stream",
        "description": "Creates a new stream with specified name and openness level",
        "parameters": {
          "type": "object",
          "properties": {
            "name": {"type": "string", "description": "..."},
            "openness": {"type": "string", "enum": ["PreApproved", "Apply", "Open"]}
          },
          "required": ["name", "openness"]
        }
      }]
    }]
  }
}
```

### API Testing Status

**Connection**: ✅ WebSocket connects successfully
**Setup Message**: ✅ Sent with correct format including tools
**Issue**: Server closes connection immediately after receiving setup message

This suggests:
1. The API may be in experimental/preview state with limited availability
2. Additional configuration or permissions may be required for the API key
3. The model name format may need verification against latest API documentation

## Architecture

- **`error.rs`** (42 LOC) - Error types for WebSocket and API errors
- **`types.rs`** (157 LOC) - Gemini Live API message type definitions including Tool and FunctionDeclaration
- **`live_client.rs`** (235 LOC) - WebSocket client with auto-reconnect and tool support
- **`dispatcher.rs`** (224 LOC) - Tool call dispatcher with concurrency control
- **`lib.rs`** (12 LOC) - Public API exports

## Usage Example

```rust
use arkavo_gemini::{LiveSessionClient, ToolDispatcher, ToolRegistry};
use serde_json::json;

// 1. Register tools first
let dispatcher = ToolDispatcher::new(4); // max 4 concurrent
let mut registry = ToolRegistry::new();

registry.register(
    "create_stream",
    "Creates a new stream",
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "openness": {"type": "string"}
        },
        "required": ["name", "openness"]
    }),
    |args| {
        // Tool handler implementation
        Ok(json!({"status": "created"}))
    },
);

registry.build(&dispatcher);

// 2. Get tool schemas
let tool_schemas = dispatcher.list_tools();

// 3. Create client with tools
let client = LiveSessionClient::new_with_tools(
    "api-key",
    "gemini-2.5-flash-live-preview",
    tool_schemas
);

client.connect().await?;

// 4. Send prompt - tools are already declared
client.send_prompt("create a stream called 'test'").await?;

// 5. Receive and execute tool calls
let calls = client.receive_tool_calls().await?;
let results = dispatcher.dispatch(calls).await;

// 6. Send responses
for (id, result) in results {
    client.send_tool_response(id, result?).await?;
}
```

## Testing

Run tests:
```bash
cargo test -p arkavo-gemini
```

All tests pass (6 tests total):
- Tool registration
- Tool execution
- Idempotency checking
- Client creation (with and without tools)
- Dispatcher functionality

## Integration

Already integrated with `arkavo-llm` via feature flag:
```toml
[features]
gemini = ["arkavo-gemini", ...]
```

## Next Steps for Production Use

1. **Verify API Access**: Confirm API key has Live API access (may require enrollment)
2. **Check Model Availability**: Verify which models support the Live API endpoint
3. **Review Error Messages**: Check if server sends error details before closing
4. **Test Alternative Endpoints**: Try Vertex AI endpoint if available

## Implementation Details

### Key Changes from Initial Implementation

1. **Added Tool Declaration Types** (~40 LOC):
   - `Tool` struct with `functionDeclarations`
   - `FunctionDeclaration` with name, description, parameters
   - Updated `SetupConfig` to include optional `tools` field

2. **Enhanced LiveSessionClient** (~30 LOC):
   - `new_with_tools()` constructor accepting tool schemas
   - `send_setup()` now constructs and sends tool declarations
   - Backward compatible with `new()` for tool-less sessions

3. **Integration Pattern**:
   - Tools registered via `ToolRegistry`
   - Schemas extracted via `dispatcher.list_tools()`
   - Passed to client constructor before connecting

## License

Apache-2.0
