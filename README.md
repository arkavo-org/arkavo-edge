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
```bash
# Interactive mode
arkavo chat

# Single query
arkavo chat --prompt "Explain this codebase"
```

AI-powered conversational interface with streaming responses and repository context. Uses Ollama with `devstral` model by default.

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
