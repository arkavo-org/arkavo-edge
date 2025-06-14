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

For iOS simulator testing capabilities, Arkavo Edge now uses a high-performance Direct FFI integration:

### Direct FFI Integration (Default)
- **850x smaller**: 19.1KB static library vs 16.2MB binary
- **500x faster**: Microsecond latency for UI interactions
- **Zero runtime dependencies**: Links directly into the Rust binary
- **Requirements**: 
  - Apple Silicon Mac (arm64/aarch64 only)
  - Xcode 15.3 or later
  - CoreSimulator framework (included with Xcode)

The Direct FFI library is automatically downloaded during CI builds. For local development:
```bash
# Download happens automatically during build
cargo build
```

### Legacy IDB Companion (Fallback)
If Direct FFI is not available, the system falls back to the traditional IDB companion:
```bash
# Install via Homebrew
brew tap facebook/fb
brew install idb-companion
```

### Important Mac App Store Note
⚠️ **The Direct FFI integration uses private Apple frameworks (CoreSimulator) and is NOT compatible with Mac App Store distribution.** This feature is intended for:
- Development environments
- CI/CD pipelines  
- Testing infrastructure

For Mac App Store releases, iOS testing features must be disabled.

## Commands

### Chat
```bash
# Interactive mode
arkavo chat

# Single query
arkavo chat --prompt "Explain this codebase"
```

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
