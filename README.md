# Arkavo Edge

AI-powered developer toolkit for secure, intelligent code transformation and testing.

## Key Features

### 🤖 AI Code Agent
- Multi-file refactoring with repository context
- Automatic commit generation
- GPU-accelerated terminal UI

### 🧠 Intelligent Test Generation
- AI understands your domain model and finds bugs you didn't think of
- Property-based testing with automatic invariant discovery
- State space exploration and chaos engineering
- MCP server for integration with Claude Code and other AI tools

### 🔒 Security First
- OpenTDF encryption on all payloads
- Local-first with Edge Vault storage
- No data leaves your control

### 🔀 Git Integration
- Native Git operations without OpenSSL dependency
- Auto-commit with AI-generated messages
- Safe operations with automatic rollback
- MCP tools for version control

## MCP Server for Claude Code

When downloaded to the project folder:
```bash
claude mcp add arkavo ./arkavo serve
```

Or configure manually in Claude Code settings:
```json
{
  "mcpServers": {
    "arkavo": {
      "command": "/path/to/arkavo",
      "args": ["serve"]
    }
  }
}
```

Then ask the AI to:
- "Find bugs in my payment processing logic"
- "What invariants should always be true in my user system?"
- "Test what happens when the network fails during checkout"
- "Explore edge cases in the authentication flow"

## iOS Testing Requirements (macOS only)

For iOS simulator testing capabilities, you'll need:

### idb_companion
The iOS Debug Bridge companion tool from Meta is required for reliable simulator UI automation:

```bash
# Install via Homebrew
brew tap facebook/fb
brew install idb-companion
```

**Note:** The macOS build can optionally embed idb_companion for distribution. See THIRD-PARTY-LICENSES.md for license information.

## Commands

### Chat

The chat command now launches with a Terminal UI (TUI) by default for an enhanced interactive experience.

```bash
# Interactive mode with Terminal UI (default)
arkavo chat

# Disable Terminal UI for classic CLI mode
arkavo chat --no-tui

# Single query (automatically uses CLI mode)
arkavo chat --prompt "Explain this codebase"

# Analyze an image
arkavo chat --prompt "What's in this screenshot?" --image screenshot.png
```

#### Terminal UI Keybindings

When using the Terminal UI mode:

- **Tab** - Switch between Chat/Code/Diff views
- **q** - Quit the application
- **↑/↓** - Scroll up/down in the current view
- **PageUp/PageDown** - Scroll by page
- **Home/End** - Jump to top/bottom
- **m** - Toggle between unified/side-by-side diff view (in Diff view)
- **n** - Toggle line numbers (in Code view)
- **n/p** - Next/Previous hunk (in Diff view)

The Terminal UI provides:
- **120fps Rendering**: Smooth, responsive UI with 8ms frame budget
- **Chat View**: Conversation history with color-coded roles
- **Code View**: Syntax-highlighted code with line numbers
- **Diff View**: Unified or side-by-side diff preview
- **Progress Indicators**: Visual feedback for long operations
- **Multi-Terminal Support**: Spawn additional terminals for parallel tasks (experimental)

AI-powered conversational interface with streaming responses and repository context. Uses Ollama with `devstral` model by default.

#### MCP Integration
The chat command automatically connects to a local MCP server (if running) to provide access to powerful tools:

```bash
# Terminal 1: Start MCP server
arkavo serve

# Terminal 2: Use chat with MCP tools
arkavo chat
```

In chat, you can:
- Type `tools` to list available MCP tools
- Use `@toolname [args]` to invoke tools directly
- Example: `@screen_capture {"name": "test1"}`

The integration is automatic - if no MCP server is running, chat falls back to LLM-only mode.

#### Vision Model Support
For UI testing with screenshots, install a vision-capable model:

```bash
# Install llava vision model (4.7 GB)
ollama pull llava:7b

# Use with screenshots
arkavo chat --prompt "What UI elements are visible?" --image screenshot.png

# Or interactively
arkavo chat
> @screenshot path/to/screenshot.png
```

**Note:** Images are limited to 10MB. Supported formats: PNG, JPEG, WebP.

### Serve
```bash
arkavo serve
```

Run as MCP server for Claude Code integration.

## License

Apache 2.0 - See LICENSE file for details.
