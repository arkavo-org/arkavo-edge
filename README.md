# Arkavo Edge

AI-powered developer toolkit for secure, intelligent code transformation and testing.

## Quick Start

Get your first AI agent running in under 5 minutes:

```bash
# Install (macOS Apple Silicon)
curl -L https://github.com/arkavo-org/arkavo-edge/releases/download/v0.21.0-alpha/arkavo-macos-aarch64.tar.gz | tar -xz
mv arkavo /usr/local/bin

# Create and run your first agent
arkavo agent init my-agent
arkavo agent run

# In another terminal, launch the UI
arkavo ui
# Open http://127.0.0.1:7700 in your browser
```

See the [Getting Started Guide](docs/GETTING_STARTED.md) for detailed instructions.

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

## iOS Testing (Optional, macOS only)

iOS simulator testing capabilities are available on macOS but require Xcode Command Line Tools.

### Requirements

**Xcode Command Line Tools** - Required for iOS simulator control and testing:
```bash
# Install Xcode Command Line Tools
xcode-select --install
```

### What happens without Xcode?

If Xcode is not installed:
- Arkavo Edge will still run normally
- iOS testing tools will be automatically disabled
- No system prompts will appear
- You'll see a message indicating iOS features are unavailable

### Embedded Tools

The macOS build includes an embedded idb_companion (iOS Debug Bridge) from Meta for reliable simulator UI automation. This tool is automatically extracted and managed by Arkavo Edge when iOS testing features are used. See THIRD-PARTY-LICENSES.md for license information.

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
