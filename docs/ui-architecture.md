# UI Architecture

Arkavo Edge provides five distinct interaction modes, including three graphical user interfaces.

## Overview

The architecture consists of five interaction modes:

1. **Terminal UI (TUI)** - Interactive terminal interface with streaming responses
2. **CEF UI** - Native Chromium-based web interface with DOM manipulation (macOS .pkg)
3. **Web UI** - HTTP/WebSocket server with browser-based interface (Homebrew)
4. **REPL (chat)** - Basic command-line REPL for simple conversations
5. **A2A Protocol** - Agent-to-agent communication for orchestration

## Current Status

| Mode | Command | Status | Package | Implementation |
|------|---------|--------|---------|----------------|
| TUI | `arkavo terminal` | ✅ Production | Both | Full ratatui implementation |
| CEF UI | `arkavo ui` (cef-ui) | ✅ Production | macOS .pkg | Full CEF integration with IPC |
| Web UI | `arkavo ui` (web-ui) | ✅ Restored | Homebrew | AgUiGateway with WebSocket |
| REPL | `arkavo chat` | ✅ Production | Both | Simple command-line interface |
| A2A | N/A | ✅ Production | Both | Protocol-based orchestration |

## New: Unified Abstraction Layer (arkavo-ui-core)

A new crate `arkavo-ui-core` provides shared abstractions and LLM integration:

**Core Traits**:
```rust
pub trait UserInterface {
    async fn initialize(&mut self) -> Result<()>;
    async fn display(&mut self, content: UIContent) -> Result<()>;
    async fn next_event(&mut self) -> Result<UIEvent>;
    fn is_running(&self) -> bool;
    async fn shutdown(self: Box<Self>) -> Result<()>;
}
```

**Unified LLM Integration**:
```rust
pub struct LlmIntegration {
    router: Router,
}
// Provides router-based model selection for all UIs
```

**Adapters** (feature-gated):
- `CEFAdapter` - Wraps CefRendererImpl (cef-ui feature)
- `WebAdapter` - Wraps AgUiGateway (web-ui feature)

Location: `crates/arkavo-ui-core/`

## Architecture Diagrams

### Command Routing (Updated)

```
┌─────────────────┐
│  arkavo CLI     │
└────────┬────────┘
         │
    ┌────┴────┬──────────┬───────────┬──────────┐
    │         │          │           │          │
    ▼         ▼          ▼           ▼          ▼
terminal    chat       ui       agent run     A2A
    │         │          │           │       Protocol
    │         │     ┌────┴─────┐     │
    │         │     │          │     │
    ▼         ▼     ▼          ▼     ▼
┌─────┐  ┌──────┐ ┌─────┐  ┌─────┐ ┌─────┐
│ TUI │  │ REPL │ │ CEF │  │ Web │ │AGUI │
│     │  │      │ │  UI │  │  UI │ │Gate │
└─────┘  └──────┘ └─────┘  └─────┘ └─────┘
                  (cef-ui) (web-ui)
                  macOS pkg Homebrew
```

**Feature-Based Selection** (compile-time):
- If `cef-ui` enabled → Routes to CEF UI
- Else if `web-ui` enabled → Routes to Web UI
- Else → Error message

### TUI Architecture

```
arkavo terminal
    │
    ├─> Parse args (--model, --temperature, etc.)
    ├─> Initialize LLM client
    ├─> Create channels (ui_tx, llm_rx)
    │
    ├─> Spawn LLM handler task
    │   └─> tokio::spawn(async move {
    │           loop {
    │               - Receive user input from ui_tx
    │               - Build conversation history
    │               - Stream LLM response
    │               - Send chunks to llm_rx with markers:
    │                   <<STREAM_START>>
    │                   [content chunks...]
    │                   <<STREAM_END>>
    │           }
    │       })
    │
    └─> Launch TUI
        └─> arkavo_terminal::run_with_string_channels(ui_tx, llm_rx)
            │
            └─> Main event loop (app.rs:594-850)
                ├─> Collect LLM responses (try_recv)
                ├─> Process streaming chunks
                ├─> Update UI state
                ├─> Render with ratatui
                └─> Handle user input (keyboard/mouse)
```

**Key Components**:

| Component | Location | Purpose |
|-----------|----------|---------|
| Terminal command handler | crates/arkavo-cli/src/commands/terminal.rs | CLI argument parsing, LLM initialization |
| TUI library core | crates/arkavo-terminal/src/lib.rs | Public API, LLM handler task |
| App state & event loop | crates/arkavo-terminal/src/app.rs | Main UI state machine |
| Chat view | crates/arkavo-terminal/src/ui/chat.rs | Message rendering |
| Code view | crates/arkavo-terminal/src/ui/code.rs | Syntax highlighting, editor |
| Diff view | crates/arkavo-terminal/src/ui/diff.rs | Git diff visualization |
| Debug view | crates/arkavo-terminal/src/ui/debug.rs | Performance metrics |
| Vim mode | crates/arkavo-terminal/src/vim/mod.rs | Modal editing support |

**Features**:
- Streaming LLM responses with real-time rendering
- Conversation history with role-based colors
- Syntax highlighting via syntect
- MCP tool integration (@toolname syntax)
- Session persistence
- Vim modal editing
- Performance monitoring (target: <8ms per frame)
- Adaptive frame rate (60fps active, 10fps idle)

### CEF UI Architecture

```
arkavo ui --prompt "Create dashboard"
    │
    ├─> Parse args (--prompt, --port)
    │
    └─> use_cef_renderer()
        │
        ├─> CefRendererImpl::new()
        │   └─> Find arkavo-cef-renderer binary
        │       ├─> /Applications/Arkavo.app
        │       ├─> /usr/local/bin
        │       ├─> ./target/debug/
        │       └─> Environment variables
        │
        ├─> CefRenderer::new()
        │   ├─> Spawn arkavo-cef-renderer process
        │   ├─> Create Unix Domain Socket
        │   │   └─> /tmp/arkavo_dom_{pid}.sock
        │   └─> Wait for connection (10s timeout)
        │
        ├─> Load prompt bar UI (2s wait)
        │
        ├─> Process initial prompt (if provided)
        │   └─> handle_prompt()
        │       ├─> Router-based model selection
        │       ├─> Stream LLM response
        │       ├─> Format as HTML
        │       └─> renderer.render(html, css, js)
        │
        └─> Event polling loop (100ms interval)
            └─> try_recv_event()
                └─> Check for event_type == "submit"
                    └─> handle_prompt(event.value)
```

**IPC Protocol** (via Unix Domain Socket):

```
┌────────────────────┐         ┌──────────────────────┐
│  Rust Main Process │◄───────►│  CEF Renderer Process│
│                    │  UDS    │                      │
│  CefRendererImpl   │         │  Chromium Engine     │
│  DOMCommandBuilder │         │  JavaScript Bridge   │
└────────────────────┘         └──────────────────────┘
         │                              │
         │ 1. Send DOM command          │
         ├─────────────────────────────►│
         │    (binary protocol)         │
         │                              │
         │ 2. Execute DOM operation     │
         │                       ┌──────┴──────┐
         │                       │ document.   │
         │                       │  querySelector│
         │                       │  .innerHTML  │
         │                       └──────┬──────┘
         │                              │
         │ 3. Send feedback/event       │
         │◄─────────────────────────────┤
         │    (success/error/event)     │
```

**DOM Operations Supported** (crates/arkavo-cef/src/dom_commands.rs):

| Operation | Opcode | Method |
|-----------|--------|--------|
| ReplaceInnerHTML | 0 | `replace_inner_html(selector, html)` |
| SetAttribute | 1 | `set_attribute(selector, attr, value)` |
| SetStyle | 2 | `set_style(selector, property, value)` |
| RemoveNode | 3 | `remove_node(selector)` |
| AppendNode | 4 | `append_node(parent, html)` |
| QuerySelector | 5 | `query_selector(selector)` |
| AddEventListener | 6 | `add_event_listener(selector, event_type)` |
| SetTextContent | 7 | `set_text_content(selector, text)` |

**Event Flow** (crates/arkavo-cef/src/protocol.rs):

```rust
DOMEvent {
    event_type: String,    // "submit", "click", "input"
    selector: String,      // CSS selector of event target
    target_id: String,     // DOM element ID
    value: String,         // Input value, button text, etc.
    data: String,          // Additional event data
}
```

**Key Components**:

| Component | Location | Purpose |
|-----------|----------|---------|
| UI command handler | crates/arkavo-cli/src/commands/ui.rs | Command parsing, CEF initialization |
| CEF renderer wrapper | crates/arkavo-agui/src/renderer/cef_renderer.rs | High-level CEF API |
| CEF process manager | crates/arkavo-cef/src/process.rs | Binary spawning, lifecycle |
| UDS transport | crates/arkavo-cef/src/uds.rs | Non-blocking IPC |
| DOM command builder | crates/arkavo-cef/src/dom_commands.rs | Type-safe DOM operations |
| Protocol definitions | crates/arkavo-cef/src/protocol.rs | Binary message format |

**Features**:
- Native Chromium window with GPU acceleration
- Direct DOM manipulation from Rust
- Event bridge for user interactions
- Rate limiting (100 concurrent commands max)
- Size validation (1MB HTML, 100KB CSS)
- Router-based LLM selection
- Real-time UI generation
- Command latency target: <5ms

### Web UI Architecture (Stub)

```
arkavo ui (without cef-ui feature)
    │
    └─> ERROR: "No UI renderer available. Build with --features cef-ui"

┌────────────────────────────────────────────────────┐
│  Existing Code (Not Used)                          │
├────────────────────────────────────────────────────┤
│                                                    │
│  AgUiGateway (crates/arkavo-agui/src/gateway.rs)  │
│  ├─> Axum HTTP server                             │
│  ├─> WebSocket connections                        │
│  ├─> mDNS agent discovery                         │
│  ├─> Budget tracking                              │
│  ├─> Dataflow visualization                       │
│  └─> Multi-client support                         │
│                                                    │
│  WebRenderer (src/renderer/web_renderer.rs)       │
│  └─> Stub implementation (no-op methods)          │
│                                                    │
└────────────────────────────────────────────────────┘
```

**What Needs to be Restored**:
1. Reconnect `ui` command to `AgUiGateway` when `web-ui` feature enabled
2. Implement actual rendering in `WebRenderer`
3. Add WebSocket event bridge for user interactions
4. Create HTML/JS frontend that connects to gateway

## Shared Components

All three UIs share the following underlying layers:

```
┌─────────────────────────────────────────────────┐
│              UI Layer (Independent)             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐     │
│  │   TUI    │  │ CEF UI   │  │ Web UI   │     │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘     │
└───────┼─────────────┼─────────────┼───────────┘
        │             │             │
┌───────┴─────────────┴─────────────┴───────────┐
│          Common LLM & Protocol Layer          │
│  ┌──────────────────────────────────────────┐ │
│  │  arkavo-llm                              │ │
│  │  ├─> LlmClient                           │ │
│  │  ├─> Streaming responses                 │ │
│  │  └─> Provider abstraction                │ │
│  └──────────────────────────────────────────┘ │
│                                                │
│  ┌──────────────────────────────────────────┐ │
│  │  arkavo-router                           │ │
│  │  ├─> Model selection                     │ │
│  │  ├─> Cost estimation                     │ │
│  │  └─> Offline fallback                    │ │
│  └──────────────────────────────────────────┘ │
│                                                │
│  ┌──────────────────────────────────────────┐ │
│  │  arkavo-mcp-tools                        │ │
│  │  ├─> Tool discovery                      │ │
│  │  ├─> Tool invocation                     │ │
│  │  └─> Result handling                     │ │
│  └──────────────────────────────────────────┘ │
│                                                │
│  ┌──────────────────────────────────────────┐ │
│  │  arkavo-memory                           │ │
│  │  ├─> Conversation history                │ │
│  │  ├─> Session management                  │ │
│  │  └─> SQLite storage                      │ │
│  └──────────────────────────────────────────┘ │
└────────────────────────────────────────────────┘
```

**Shared Components**:

| Component | Purpose | Used By |
|-----------|---------|---------|
| arkavo-llm | LLM client abstraction | All UIs |
| arkavo-router | Model selection & routing | CEF UI, (TUI planned) |
| arkavo-mcp-tools | MCP tool integration | TUI, CEF UI |
| arkavo-memory | Conversation persistence | TUI, CEF UI |
| arkavo-protocol | AG-UI protocol types | CEF UI, Web UI gateway |
| arkavo-ui-generator | Prompt-to-UI generation | CEF UI, Web UI gateway |
| arkavo-budget | Cost tracking | Web UI gateway |
| arkavo-dataflow | Task flow visualization | Web UI gateway |

## Pluggable UI Abstraction

### Current Abstraction (Limited)

The `UiRenderer` trait exists but only applies to HTML-based UIs (crates/arkavo-agui/src/renderer/mod.rs:10-23):

```rust
#[async_trait]
pub trait UiRenderer: Send + Sync {
    async fn render(&mut self, html: &str, css: &str, js: &str) -> Result<()>;
    async fn update_element(&mut self, selector: &str, html: &str) -> Result<()>;
    async fn set_style(&mut self, selector: &str, property: &str, value: &str) -> Result<()>;
    async fn add_event_listener(&mut self, selector: &str, event_type: &str) -> Result<()>;
    fn is_running(&self) -> bool;
    async fn shutdown(self: Box<Self>) -> Result<()>;
}
```

**Limitations**:
- Only supports DOM-based rendering (HTML/CSS/JS)
- TUI doesn't implement this trait (incompatible paradigm)
- Tightly coupled to web rendering model
- No abstraction for streaming responses
- No abstraction for user input events

### Proposed Pluggable Architecture

To support future UI plugins, we need a more general abstraction:

```rust
// Core UI trait that all UIs must implement
#[async_trait]
pub trait UserInterface: Send + Sync {
    /// Initialize the UI and return when ready
    async fn initialize(&mut self) -> Result<()>;

    /// Display content to the user
    async fn display(&mut self, content: UIContent) -> Result<()>;

    /// Wait for next user input event
    async fn next_event(&mut self) -> Result<UIEvent>;

    /// Check if UI is still running
    fn is_running(&self) -> bool;

    /// Clean shutdown
    async fn shutdown(self: Box<Self>) -> Result<()>;
}

// Content types that can be displayed
pub enum UIContent {
    Text(String),
    Markdown(String),
    Html { html: String, css: String, js: String },
    StreamChunk { content: String, is_complete: bool },
    Error(String),
    StatusUpdate(String),
}

// User interaction events
pub enum UIEvent {
    TextInput(String),
    ButtonClick { id: String, label: String },
    FormSubmit { data: HashMap<String, String> },
    KeyPress(KeyCode),
    Shutdown,
}
```

**Adapter Pattern for Existing UIs**:

```
┌──────────────────────────────────────────────┐
│            UserInterface Trait               │
└─────────────────┬────────────────────────────┘
                  │
        ┌─────────┼─────────┬──────────────────┐
        │                   │                  │
        ▼                   ▼                  ▼
┌──────────────┐   ┌──────────────┐   ┌──────────────┐
│ TUIAdapter   │   │ CEFAdapter   │   │ WebAdapter   │
│              │   │              │   │              │
│ Wraps        │   │ Wraps        │   │ Wraps        │
│ arkavo-      │   │ CefRenderer  │   │ AgUiGateway  │
│ terminal     │   │ Impl         │   │              │
└──────────────┘   └──────────────┘   └──────────────┘
```

**Command Routing with Plugin Support**:

```rust
// In commands/ui.rs
pub fn execute(args: &[String]) -> Result<()> {
    let ui_type = determine_ui_type(args)?;  // --ui-type flag or feature detection

    let mut ui: Box<dyn UserInterface> = match ui_type {
        UIType::Terminal => Box::new(TUIAdapter::new()),
        UIType::Cef => Box::new(CEFAdapter::new()),
        UIType::Web => Box::new(WebAdapter::new(port)),
        UIType::Custom(name) => load_plugin(&name)?,  // Future plugin support
    };

    ui.initialize().await?;

    // Unified event loop for all UI types
    loop {
        match ui.next_event().await? {
            UIEvent::TextInput(text) => {
                let response = handle_llm_request(&text).await?;
                ui.display(UIContent::StreamChunk { content: response, is_complete: true }).await?;
            },
            UIEvent::Shutdown => break,
            _ => { /* handle other events */ }
        }
    }

    ui.shutdown().await?;
}
```

## Feature Flags

| Feature | Crate | Default | Purpose |
|---------|-------|---------|---------|
| `web-ui` | arkavo-agui | ✅ | Enable WebRenderer (currently stub) |
| `cef-ui` | arkavo-agui | ❌ | Enable CEF-based native UI |
| `cef-ui` | arkavo-cli | ❌ | Forward to arkavo-agui |
| `cef-ui` | arkavo | ❌ | Forward to arkavo-cli |

**Build Commands**:

```bash
# Build with CEF UI (macOS only)
cargo build --features cef-ui

# Build without any UI (CLI only)
cargo build --no-default-features

# Build with web UI (when implemented)
cargo build --features web-ui
```

## File Organization

```
crates/
├─ arkavo-cli/
│  └─ src/commands/
│     ├─ terminal.rs       # TUI command handler
│     ├─ ui.rs             # CEF/Web UI command handler
│     └─ chat.rs           # Simple chat mode (no UI)
│
├─ arkavo-terminal/        # TUI implementation
│  └─ src/
│     ├─ lib.rs            # Public API, LLM handler
│     ├─ app.rs            # Main event loop
│     ├─ ui/               # Rendering modules
│     │  ├─ chat.rs
│     │  ├─ code.rs
│     │  ├─ diff.rs
│     │  └─ debug.rs
│     └─ vim/              # Vim mode
│
├─ arkavo-agui/            # HTML-based UI implementations
│  └─ src/
│     ├─ gateway.rs        # AgUiGateway (Web server)
│     ├─ ui_handler.rs     # WebSocket/SSE handlers
│     └─ renderer/
│        ├─ mod.rs         # UiRenderer trait
│        ├─ web_renderer.rs    # Stub
│        └─ cef_renderer.rs    # Full implementation
│
└─ arkavo-cef/             # CEF process management
   └─ src/
      ├─ lib.rs            # CefRenderer API
      ├─ process.rs        # Binary spawning
      ├─ uds.rs            # Unix socket IPC
      ├─ dom_commands.rs   # DOM operation builder
      └─ protocol.rs       # Binary message format
```

## Recommendations

### Immediate Actions

1. **Restore Web UI Functionality**
   - Add routing in `ui.rs` to detect `web-ui` feature
   - Call `AgUiGateway::start()` when web-ui enabled but cef-ui disabled
   - Implement actual rendering in `WebRenderer`

2. **Clarify Feature Precedence**
   - Document which UI takes precedence when multiple features enabled
   - Add `--ui-type` CLI flag to explicitly choose UI

3. **Document Migration Path**
   - Create guide for users affected by web UI removal
   - Explain how to use CEF UI or TUI as alternatives

### Long-term Architecture

1. **Unified UI Abstraction**
   - Implement `UserInterface` trait as proposed above
   - Create adapter wrappers for existing UIs
   - Standardize event handling across all UIs

2. **Plugin System**
   - Design plugin discovery mechanism (dylib loading)
   - Define plugin API versioning
   - Create example plugin template

3. **Configuration Layer**
   - Add UI preference to user config
   - Support UI-specific settings (port, theme, keybindings)
   - Allow per-command UI override

4. **Testing Strategy**
   - Add UI integration tests that work with all implementations
   - Create mock UI for testing command routing
   - Add performance benchmarks for each UI type

## Performance Characteristics

| UI Type | Startup Time | Response Latency | Memory Usage | CPU Usage |
|---------|--------------|------------------|--------------|-----------|
| TUI | <100ms | <8ms per frame | ~50MB | Low (adaptive FPS) |
| CEF UI | ~2s | <5ms IPC | ~200MB | Medium (Chromium) |
| Web UI | <500ms | Variable (network) | ~100MB | Low (server only) |

## Comparison Matrix

| Feature | TUI | CEF UI | Web UI |
|---------|-----|--------|--------|
| Platform | macOS, Linux, Windows | macOS, Linux | All (browser-based) |
| Remote Access | ❌ | ❌ | ✅ |
| GPU Acceleration | ❌ | ✅ | ✅ (browser) |
| Syntax Highlighting | ✅ syntect | ✅ browser | ✅ browser |
| Streaming Response | ✅ Real-time | ✅ Real-time | ✅ WebSocket/SSE |
| Multi-User | ❌ | ❌ | ✅ |
| Offline | ✅ | ✅ | ✅ (local server) |
| Installation | Binary only | Binary + CEF framework | Binary + browser |
| Resource Usage | Low | Medium-High | Low (server) |
| Customization | Keybindings | HTML/CSS/JS | HTML/CSS/JS |
| MCP Integration | ✅ | ✅ | ✅ |

## Implementation Details

### Restoring Web UI (Completed)

The Web UI was broken in commit 30698e9 when the `ui` command was changed to only route to CEF. The restoration involved:

1. **Added Web UI Routing** (crates/arkavo-cli/src/commands/ui.rs):
   ```rust
   #[cfg(all(feature = "web-ui", not(feature = "cef-ui")))]
   {
       println!("Starting Arkavo UI with web renderer...");
       use_web_gateway(port, initial_prompt).await
   }
   ```

2. **Implemented WebRenderer Broadcasting** (crates/arkavo-agui/src/renderer/web_renderer.rs):
   - Added `broadcast_tx` channel for WebSocket updates
   - Implemented actual rendering (was stub before)
   - Broadcasts JSON messages: `render`, `update_element`, `set_style`

3. **Created use_web_gateway Function**:
   - Initializes `AgUiGateway` with port and optional prompt
   - Calls `gateway.start()` to launch Axum server
   - Available at http://localhost:7700 by default

### arkavo-ui-core Crate

**Purpose**: Shared UI abstractions and unified LLM integration

**Structure**:
```
crates/arkavo-ui-core/
├─ src/
│  ├─ lib.rs                  # Public exports
│  ├─ types.rs                # UserInterface trait, UIContent, UIEvent
│  ├─ llm_integration.rs      # Router-based LLM client creation
│  └─ adapters/
│     ├─ mod.rs
│     ├─ cef.rs               # CEFAdapter (cef-ui feature)
│     └─ web.rs               # WebAdapter (web-ui feature)
```

**Key Features**:
- `UserInterface` trait for pluggable UI implementations
- `LlmIntegration` for consistent router-based model selection
- Feature-gated adapters for CEF and Web UIs
- HTML escaping for safe rendering

### Feature Flags (Updated)

**Root Cargo.toml**:
- Added `arkavo-ui-core` to workspace members

**arkavo-cli/Cargo.toml**:
```toml
[features]
default = ["memory", "mdns", "test-harness", "llm-remote", "llama-cpp", "web-ui"]
cef-ui = ["arkavo-agui/cef-ui", "arkavo-ui-core/cef-ui"]
web-ui = ["arkavo-agui/web-ui", "arkavo-ui-core/web-ui"]
llama-cpp = ["arkavo-llm/llama-cpp", "arkavo-agui/llama-cpp", "arkavo-ui-core/llama-cpp"]
```

**Build Commands**:
```bash
# Homebrew build (web UI)
cargo build --release --features web-ui,llama-cpp,memory,mdns

# macOS .pkg build (CEF UI)
cargo build --release --features cef-ui,llama-cpp,memory,mdns

# Both features (CEF takes precedence)
cargo build --release --features cef-ui,web-ui
```

### Files Modified

1. **New Crate**: `crates/arkavo-ui-core/` (6 files)
2. **Modified**: `crates/arkavo-cli/src/commands/ui.rs` (+17 lines)
3. **Modified**: `crates/arkavo-agui/src/renderer/web_renderer.rs` (from stub to functional)
4. **Updated**: `Cargo.toml` (workspace members)
5. **Updated**: `crates/arkavo-cli/Cargo.toml` (features and dependencies)
6. **Updated**: `docs/ui-architecture.md` (this document)

### Next Steps (Optional)

1. **Unified LLM Integration for TUI**:
   - Update `terminal.rs` to use `LlmIntegration` instead of direct `initialize_llm_client()`
   - Benefits from router's cost estimation and reasoning

2. **Complete Adapter Implementation**:
   - Currently adapters exist but aren't used by command handlers
   - Future refactor could route through `UserInterface` trait

3. **WebSocket Event Bridge**:
   - Web UI needs bidirectional event handling
   - Add WebSocket endpoint for user interactions

## Related Documentation

- [MCP Integration](mcp-integration.md) - Tool integration across UIs
- [LLM Providers](llm-providers.md) - Model selection and streaming
- [Session Management](session-management.md) - Conversation persistence
- [Build Configuration](build-configuration.md) - Feature flags and compilation
