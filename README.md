# Arkavo Edge

AI-powered developer toolkit for secure, intelligent code transformation and testing.

## Quick Start

```bash
# Start interactive agent
arkavo chat

# Run intelligent tests
arkavo test --explore
```

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

Configure in Claude Code settings:
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

## Commands

- `arkavo chat` - Interactive AI agent
- `arkavo test` - Run intelligent tests
- `arkavo plan` - Preview changes
- `arkavo apply` - Execute changes
- `arkavo serve` - MCP server mode

## License

Apache 2.0 - See LICENSE file for details.
