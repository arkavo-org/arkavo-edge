# arkavo-agui

AG-UI (Agentic GUI) protocol implementation and web gateway for Arkavo Edge.

## Features

- **AI-Driven UI Generation**: Prompt-to-UI system that generates production-ready web components
- **Real-time Streaming**: Streams HTML, CSS, and JavaScript from Gemini LLM
- **Auto-start Mode**: Automatically begins UI generation when started with `--prompt` flag
- **WebSocket Protocol**: Real-time bidirectional communication for live updates
- **Status Monitoring**: System health, MCP tools, and remote LLM connection monitoring
- **AG-UI Protocol**: Full implementation of the Agentic GUI event protocol

## Quick Start

### Basic Usage

Start the UI generator:

```bash
cargo run --bin arkavo -- ui
```

Then open http://127.0.0.1:7700 and enter a prompt to generate UI components.

### Auto-start with Prompt

Start with automatic UI generation:

```bash
export GEMINI_API_KEY=your_api_key
export GEMINI_MODEL=gemini-2.5-pro  # optional, defaults to gemini-2.5-pro

cargo run --bin arkavo -- ui --prompt "Build a calculator"
```

The system will:
1. Launch the web interface
2. Automatically plan the UI components
3. Generate each component using Gemini
4. Stream the generated code to the browser

## Architecture

### Components

- **Gateway** (`gateway.rs`): WebSocket server and HTTP endpoints
- **UI Planner** (`arkavo-ui-generator/planner.rs`): Breaks down prompts into component plans
- **Streaming Generator** (`arkavo-ui-generator/streaming.rs`): Generates code using Gemini LLM
- **Frontend** (`static/shell.html`, `static/toolbar.js`): Web interface for AI-driven UI generation

### Event Flow

1. User submits prompt (or auto-submitted via `--prompt`)
2. UiPlanner analyzes prompt and creates component plan
3. Plan sent to frontend via WebSocket
4. User/system triggers generation for each component
5. StreamingGenerator calls Gemini API
6. Generated HTML/CSS/JS streamed back to frontend
7. Components rendered in sandbox

## Configuration

### Environment Variables

- `GEMINI_API_KEY`: Required for UI generation
- `GEMINI_MODEL`: LLM model to use (default: `gemini-2.5-pro`)

### Features

- `mdns`: Enable mDNS service discovery (enabled by default)

## Testing

### Integration Tests

Comprehensive E2E tests with browser screenshot validation:

```bash
# Set Gemini API key
export GEMINI_API_KEY="your-api-key"

# Run all integration tests
cd crates/arkavo-ui-generator
./run_integration_tests.sh

# Run specific test
cargo test --test integration_test test_calculator_ui_generation -- --ignored --nocapture
```

Tests generate screenshots in `target/test-output/` for visual validation.

See [arkavo-ui-generator/TESTING.md](../arkavo-ui-generator/TESTING.md) for complete testing guide.

### Development

Build and run tests:

```bash
cargo build -p arkavo-agui
cargo test -p arkavo-agui
cargo test -p arkavo-ui-generator  # Unit tests
```

## MCP Tool Integration (TODO)

### Current Status: Not Integrated

The CEF UI currently **does not use Router's MCP tool integration** or quality gate validation.

**Current Flow** (`gateway.rs` lines 962-967):
```rust
let router = Arc::new(Router::new().await?);
let planner = UiPlanner::new(router);
let plan = planner.plan(&cleaned_text).await?;  // ❌ No tools!
```

**Issues:**
- LLM doesn't receive MCP tool definitions
- Cannot call tools like `filesystem__list_directory`, `github_org_repos`, etc.
- No quality validation (hallucinated tools, refusals)
- No automatic model escalation on poor responses

### How to Add MCP Tools

**Step 1: Initialize ToolRegistry in Gateway** (`gateway.rs` ~line 960):
```rust
use arkavo_mcp_tools::ToolRegistry;

// In handle_event() for SubmitPrompt
let tool_registry = if let Some(mcp_client) = &self.mcp_client {
    ToolRegistry::from_mcp_connection(mcp_client.clone())?
} else {
    ToolRegistry::new()  // Use default hardcoded tools
};
```

**Step 2: Update Planner to Accept Tools** (`planner.rs`):
```rust
// Add new method to UiPlanner
pub async fn plan_with_tools(
    &self,
    prompt: &str,
    tool_registry: Option<&ToolRegistry>,
) -> Result<String> {
    let messages = vec![Message {
        role: Role::User,
        content: self.build_planning_prompt(prompt),
        images: None,
    }];

    // Use Router's quality gate instead of direct provider call
    let response = self.router.route_with_quality_gate(
        prompt,
        messages,
        tool_registry,
        3,  // Max retries
    ).await?;

    Ok(response.content)
}
```

**Step 3: Update Gateway to Use New Method**:
```rust
// Replace planner.plan() with planner.plan_with_tools()
let plan = planner.plan_with_tools(&cleaned_text, Some(&tool_registry)).await?;
```

**Step 4: Add Tool Execution Loop**:
```rust
// After getting response with tool_calls
if !response.tool_calls.is_empty() {
    for tool_call in &response.tool_calls {
        if let Some(tool) = tool_registry.get(&tool_call.tool_name) {
            let result = tool.execute(tool_call.arguments.clone()).await?;
            // Feed result back to LLM for refinement
        }
    }
}
```

### Integration Points

| Location | File | Lines | Change Required |
|----------|------|-------|-----------------|
| **SubmitPrompt Handler** | `gateway.rs` | 962-967 | Add ToolRegistry, call `plan_with_tools()` |
| **ApplyPart Handler** | `gateway.rs` | 1109-1117 | Add ToolRegistry to streaming generator |
| **Planner** | `../arkavo-ui-generator/src/planner.rs` | 41-62 | Add `plan_with_tools()` method |
| **Streaming** | `../arkavo-ui-generator/src/streaming.rs` | 68-180 | Add `generate_part_with_tools()` method |

### Benefits After Integration

✅ **Access to MCP tools** - UI generator can call filesystem, GitHub, browser tools
✅ **Quality validation** - Prevents hallucinated tools and bad responses
✅ **Auto-escalation** - Automatically retries with better models if needed
✅ **Better UX** - Tool execution feedback shown in UI
✅ **Cost optimization** - Tries local models first, escalates to cloud only if needed

### Example: Using Tools in UI Generation

```bash
# After integration, this will work:
arkavo ui --prompt "Show me the repository structure and generate a file browser UI"

# The LLM will:
# 1. Call filesystem__list_directory tool to get actual file list
# 2. Generate UI based on real data (not hallucinated)
# 3. If response quality is poor, automatically retry with better model
```

### See Also

- `crates/arkavo-router/README.md` - Router quality gate documentation
- `crates/arkavo-ui-generator/README.md` - Planner/streaming implementation details

## Dependencies

- `arkavo-ui-generator`: UI planning and code generation
- `arkavo-gemini`: Gemini API client
- `arkavo-router`: LLM routing (includes quality gate for tool validation)
- `arkavo-events`: Event system
- `arkavo-mcp-tools`: MCP tool registry (not currently integrated)
- `warp`: HTTP and WebSocket server
