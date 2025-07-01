# CLAUDE.md - Arkavo Terminal UI

This file provides comprehensive guidance for AI assistants working with the Arkavo Terminal UI codebase. It documents the architecture, components, expected behaviors, and testing requirements to prevent recurring TUI bugs.

## Overview

Arkavo Terminal is a GPU-accelerated terminal UI that provides:
- Multi-window chat interface with streaming LLM responses
- Code editor integration (Helix)
- Git diff preview and visualization
- Debug telemetry and performance monitoring
- Dataflow visualization
- Vim-style keyboard navigation
- MCP (Model Context Protocol) tool integration

## Architecture

### Core Components

1. **App (`src/app.rs`)**
   - Main application state management
   - View mode coordination (Chat, Code, Diff, Debug, Dataflow)
   - Layout modes (Tabbed, Portrait)
   - Focus management between panes
   - Provider configuration (Ollama, OpenAI, Anthropic)

2. **Event Handler (`src/event.rs`)**
   - Keyboard and mouse event processing
   - Non-blocking event handling with configurable tick rate
   - Event routing to appropriate views

3. **Views (`src/ui/`)**
   - `chat.rs` - Chat interface with streaming responses
   - `code.rs` - Code editor view with syntax highlighting
   - `diff.rs` - Git diff visualization
   - `debug.rs` - Performance metrics and telemetry
   - `dataflow.rs` - Task flow visualization
   - `task_manager.rs` - Multi-terminal task management

4. **Vim State (`src/vim.rs`)**
   - Modal editing support (Normal, Insert, Visual, Command)
   - Key mapping and command processing
   - Clipboard integration

5. **Renderer (`src/renderer.rs`)**
   - Optimized frame rendering
   - Performance budgeting (<8ms target)
   - Incremental updates

## Key Bindings

### Global Navigation
- `Tab` / `Shift+Tab` - Cycle between view modes
- `Ctrl+q` - Quit application

### Vim Mode
- `i` - Enter Insert mode
- `Esc` - Return to Normal mode
- `v` - Enter Visual mode
- `:` - Enter Command mode
- `h/j/k/l` - Navigation (left/down/up/right)
- `g/G` - Go to top/bottom
- `Ctrl+u/d` - Page up/down
- `y` - Yank (copy) in Visual mode
- `p` - Paste

### Chat View
- `Enter` - Send message (Insert mode)
- `m` - Open model selection
- `Up/Down` - Navigate model list
- `Esc` - Cancel model selection

### Code View
- `Ctrl+e` - Open external editor (Helix)
- Standard vim navigation applies

## Critical State Management

### Model Connection States

The TUI manages multiple LLM provider connections with these states:

```rust
enum ProviderStatus {
    Connected,      // Active connection, models available
    Disconnected,   // Connection lost, retry possible
    NotConfigured,  // No credentials/URL configured
    Error(String),  // Specific error condition
}
```

### Window Management Per Model

Each model connection maintains its own:
- Chat history
- Streaming state
- Error messages
- Task queue

### Configuration Modes

```rust
enum ConfigurationMode {
    None,                               // Normal operation
    OllamaServer { input, testing },    // Ollama server URL input
    OpenAIKey { input },                // API key input
    AnthropicKey { input },             // API key input
}
```

## Testing Requirements

### TUI Testing Tools

The `arkavo-test` crate provides MCP tools for automated TUI testing:

1. **tui_keyboard** - Send keyboard input
   - Supports key combinations with modifiers
   - Text typing and shortcuts
   - Platform-specific implementations (macOS/Linux)

2. **tui_screenshot** - Capture terminal state
   - Text format (with/without ANSI colors)
   - Image format (PNG base64)
   - Window selection by title

3. **tui_interaction** - Combined keyboard/screenshot
   - Type and verify operations
   - Navigation with verification
   - State assertions

4. **tui_harness** - Session management
   - Start/stop TUI sessions
   - I/O capture and monitoring
   - Process lifecycle management

### Regression Test Scenarios

1. **Key Binding Tests**
   ```rust
   // Test vim mode transitions
   tui_keyboard { action: "key", key: "i" }        // Enter insert mode
   tui_screenshot { format: "text" }               // Verify mode indicator
   tui_keyboard { action: "key", key: "escape" }   // Return to normal mode
   tui_screenshot { format: "text" }               // Verify mode change
   ```

2. **Model Connection Tests**
   ```rust
   // Test model selection
   tui_keyboard { action: "key", key: "m" }        // Open model selector
   tui_screenshot { format: "text" }               // Verify model list
   tui_keyboard { action: "key", key: "down" }     // Navigate models
   tui_keyboard { action: "key", key: "enter" }    // Select model
   ```

3. **Window Focus Tests**
   ```rust
   // Test pane focus in portrait mode
   tui_keyboard { action: "shortcut", shortcut: "t", modifiers: ["ctrl"] }  // Switch to portrait
   tui_keyboard { action: "shortcut", shortcut: "f", modifiers: ["ctrl"] }  // Toggle focus
   tui_screenshot { format: "text" }                                        // Verify focus indicator
   ```

4. **Chat Streaming Tests**
   ```rust
   // Test message sending and streaming
   tui_keyboard { action: "key", key: "i" }                    // Insert mode
   tui_keyboard { action: "text", text: "Hello, assistant" }   // Type message
   tui_keyboard { action: "key", key: "enter" }                 // Send
   // Wait and capture streaming response
   ```

## Common Bug Patterns and Prevention

### 1. Mode Confusion
**Issue**: Vim mode state not properly reflected in UI
**Prevention**: 
- Always update `vim_state` before rendering
- Ensure mode indicator is visible in status line
- Test mode transitions with screenshot verification

### 2. Focus Loss
**Issue**: Focus jumps unexpectedly between panes
**Prevention**:
- Centralize focus management in `App::handle_focus_change()`
- Validate focus target before switching
- Test all focus transitions in both layout modes

### 3. Model Connection Drops
**Issue**: Model connections silently fail
**Prevention**:
- Implement connection health checks
- Display connection status in UI
- Provide clear error messages with recovery options

### 4. Streaming State Corruption
**Issue**: Multiple streaming responses overlap
**Prevention**:
- Use task_id to track individual requests
- Cancel previous streams before starting new ones
- Test concurrent model requests

### 5. Key Binding Conflicts
**Issue**: Keys perform unexpected actions
**Prevention**:
- Document all key bindings in this file
- Check for conflicts when adding new bindings
- Test key combinations with modifiers

## Performance Considerations

1. **Frame Budget**: Target <8ms render time
2. **Event Processing**: Non-blocking with 10ms tick rate
3. **Syntax Highlighting**: Lazy loading, cache parsed results
4. **Scrolling**: Virtualized rendering for large content

## MCP Integration

The Terminal UI integrates with MCP tools through:
- Tool discovery on startup
- `@toolname` syntax in chat messages
- Automatic tool result display
- Error handling for failed tool calls

## Debug Features

The Debug view (`F12`) shows:
- Frame render times
- Event processing metrics
- Memory usage
- Active connections
- MCP tool call history

## Testing Checklist

Before committing TUI changes:

- [ ] Run TUI regression tests using MCP tools
- [ ] Test all view modes (Chat, Code, Diff, Debug, Dataflow)
- [ ] Test both layout modes (Tabbed, Portrait)
- [ ] Verify vim mode transitions
- [ ] Test model selection and switching
- [ ] Verify focus management
- [ ] Test error conditions (connection loss, invalid input)
- [ ] Check performance metrics in Debug view
- [ ] Test on both macOS and Linux