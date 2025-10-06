# arkavo-gemini

Gemini API integration for Arkavo Edge with function calling support.

## Features

- ✅ REST API client with function calling (`generateContent`)
- ✅ WebSocket Live API client (audio-focused, experimental)
- ✅ Tool call dispatcher with configurable concurrency limits
- ✅ Idempotency via `requestId` deduplication
- ✅ Schema validation for tool arguments
- ✅ Async tool execution with semaphore-based limiting

## Implementation Status

**Code**: Complete (~900 LOC across 6 files, all <400 LOC)
**Tests**: All passing (6 unit + integration tests)
**Quality**: Clippy clean, formatted

## API Comparison

### REST API (Recommended for Text + Tools)

**Use Case**: Text-based interactions with function calling

**Pros**:
- ✅ Reliable function calling support
- ✅ Text-native responses
- ✅ Works with all Gemini models
- ✅ Simple HTTP requests
- ✅ Proven, stable API

**Cons**:
- ❌ Higher latency than WebSocket
- ❌ No real-time streaming

**Example**:
```rust
use arkavo_gemini::{RestClient, FunctionDeclaration};
use serde_json::json;

let client = RestClient::new("api-key", "models/gemini-2.0-flash-exp");

let tools = vec![FunctionDeclaration {
    name: "create_stream".to_string(),
    description: "Creates a new stream".to_string(),
    parameters: json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "openness": {"type": "string"}
        },
        "required": ["name", "openness"]
    }),
}];

let (text, calls) = client
    .generate_content("Create a stream called 'test'", Some(tools))
    .await?;

// Execute tool calls
let results = dispatcher.dispatch(calls).await;
```

### Live API (Audio-Focused, Experimental)

**Use Case**: Real-time audio/video conversations

**Pros**:
- ✅ Low-latency WebSocket connection
- ✅ Real-time audio streaming
- ✅ Supports video input

**Cons**:
- ❌ Requires audio modality
- ❌ Limited model availability (`gemini-2.5-flash-native-audio-preview-09-2025`)
- ❌ TEXT-only mode not fully supported
- ❌ Beta/experimental status

**Status**: The Live API WebSocket client is implemented but requires AUDIO response modality. For text-based tool calling, use the REST API instead.

## Architecture

- **`error.rs`** (42 LOC) - Error types for WebSocket and API errors
- **`types.rs`** (180 LOC) - Message type definitions for both APIs
- **`rest_client.rs`** (155 LOC) - REST API client for text-based tool calling
- **`live_client.rs`** (270 LOC) - WebSocket client for audio conversations
- **`dispatcher.rs`** (224 LOC) - Tool call dispatcher with concurrency control
- **`lib.rs`** (15 LOC) - Public API exports

## Usage Example (REST API with Tools)

```rust
use arkavo_gemini::{RestClient, ToolDispatcher, ToolRegistry, FunctionDeclaration};
use serde_json::json;

// 1. Register tools
let dispatcher = ToolDispatcher::new(4);
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
        let name = args["name"].as_str().unwrap();
        Ok(json!({
            "status": "created",
            "name": name
        }))
    },
);

registry.build(&dispatcher);

// 2. Create REST client
let client = RestClient::new("api-key", "models/gemini-2.0-flash-exp");

// 3. Convert tool schemas to FunctionDeclarations
let tools: Vec<FunctionDeclaration> = dispatcher
    .list_tools()
    .iter()
    .map(|t| FunctionDeclaration {
        name: t["name"].as_str().unwrap().to_string(),
        description: t["description"].as_str().unwrap().to_string(),
        parameters: t["parameters"].clone(),
    })
    .collect();

// 4. Send request with tools
let (text, calls) = client
    .generate_content("Create a stream called 'test'", Some(tools))
    .await?;

// 5. Execute tool calls
if !calls.is_empty() {
    let results = dispatcher.dispatch(calls).await;
    for (id, result) in results {
        println!("Tool {} result: {:?}", id, result);
    }
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
- Client creation
- Dispatcher functionality

Run the REST API example:
```bash
GEMINI_API_KEY=your-key cargo run -p arkavo-gemini --example rest_tool_test
```

## Integration

Integrated with `arkavo-llm` via feature flag:
```toml
[features]
gemini = ["arkavo-gemini", ...]
```

## Recommended Approach for Issue #249

For **text-based fast tool calling** (sub-second latency):

1. **Use REST API** (`generateContent`) - Proven, reliable function calling
2. **Tool Dispatcher** - Concurrent execution with semaphore limiting
3. **Local Gemma-3** - For routing/fallback (separate implementation)

The Live API is audio-focused and requires AUDIO modality, making it unsuitable for pure text-based tool calling workflows.

## Key Findings

### Live API vs REST API

**Live API (WebSocket)**:
- Designed for **audio/video conversations**
- Requires `AUDIO` response modality
- TEXT-only mode is not fully supported for `gemini-2.0-flash-exp`
- Server accepts TEXT setup but doesn't generate responses
- Best for: Real-time voice interactions

**REST API (HTTP)**:
- Designed for **text-based interactions**
- Full function calling support
- Works with all Gemini models
- Reliable and proven
- Best for: Text-based tool calling workflows

### Implementation Details

The crate provides both APIs:

1. **RestClient** - HTTP-based `generateContent` with function calling (recommended for text)
2. **LiveSessionClient** - WebSocket-based Live API (for audio conversations)
3. **ToolDispatcher** - Concurrent tool execution with idempotency
4. **ToolRegistry** - Fluent API for registering tools

## License

Apache-2.0
