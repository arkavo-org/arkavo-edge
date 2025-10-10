# arkavo-agui

AG-UI (Agentic GUI) protocol implementation and web gateway for Arkavo Edge.

## Features

- **Blank Canvas UI Generator**: Prompt-to-UI system that generates production-ready web components
- **Real-time UI Generation**: Streams HTML, CSS, and JavaScript from Gemini LLM
- **Auto-start Mode**: Automatically begins UI generation when started with `--prompt` flag
- **WebSocket Protocol**: Real-time bidirectional communication for live updates
- **Status Dashboard**: System health, MCP tools, and remote LLM connection monitoring
- **AG-UI Protocol**: Full implementation of the Agentic GUI event protocol

## Quick Start

### Basic Usage

Start the UI generator in blank canvas mode:

```bash
cargo run --bin arkavo -- ui --blank
```

Then open http://127.0.0.1:7700 and enter a prompt to generate UI components.

### Auto-start with Prompt

Start with automatic UI generation:

```bash
export GEMINI_API_KEY=your_api_key
export GEMINI_MODEL=gemini-2.5-pro  # optional, defaults to gemini-2.5-pro

cargo run --bin arkavo -- ui --blank --prompt "Build a calculator"
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
- **Frontend** (`static/shell.html`, `static/toolbar.js`): Web interface for the blank canvas

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

## Dependencies

- `arkavo-ui-generator`: UI planning and code generation
- `arkavo-gemini`: Gemini API client
- `arkavo-router`: LLM routing
- `arkavo-events`: Event system
- `arkavo-mcp-tools`: MCP tool registry
- `warp`: HTTP and WebSocket server
