# Arkavo Edge v0.30.0 Release Notes

## 🎉 Claude Code SDK Integration

This release introduces a powerful new capability: **Claude Code SDK integration**, bringing Claude's advanced agentic coding capabilities directly into Arkavo Edge.

### ✨ Key Features

#### Claude Code Capability
- **Real-time code generation** with streaming iterator pattern
- **Multi-provider support**: Works with both Anthropic Claude and DeepSeek APIs
- **Secure workspace sandboxing** with configurable file access patterns
- **Policy-controlled tool execution** with dual-layer security enforcement
- **Budget tracking** with token usage management and cost estimation

#### Technical Implementation
- New `arkavo-claude-code` crate implementing MCP Tool trait
- Node.js bridge using JSON-RPC for subprocess communication
- Event mapping from SDK events to Arkavo's event bus
- Runtime dependency checking for graceful Homebrew distribution
- Zero-configuration compliance with dynamic path discovery

### 🔧 Configuration

Configure Claude Code in your agent's `AGENTS.md`:

```yaml
claude_code:
  enabled: true
  workspace_root: ./workspace
  budget_tokens: 200000
  
  tools:
    read: true
    write: true
    exec: false      # Disabled for safety by default
    web_search: true
```

### 🚀 Getting Started

1. **Install prerequisites**:
   ```bash
   # Install Node.js >= 18.0.0
   brew install node
   
   # Install Claude Code SDK
   npm install -g @anthropic-ai/claude-code
   ```

2. **Set up credentials**:
   - For Claude: Set `ANTHROPIC_API_KEY` environment variable
   - For DeepSeek: Configure `anthropic.auth_token` in AGENTS.md

3. **Run the example agent**:
   ```bash
   cd examples/claude-code-agent
   ./launch_agent.sh
   ```

### 🔒 Security Features

- **Workspace sandboxing**: Restrict file access with allow/deny glob patterns
- **Tool permission checks**: Policy enforcement before tool execution
- **Budget limits**: Prevent excessive API usage with configurable limits
- **Credential protection**: API keys secured via .gitignore and environment variables

### 🛠️ Development Improvements

- **Zero configuration**: Dynamic discovery of system installations
- **Runtime dependency checking**: Graceful handling when Claude SDK not installed
- **Streaming architecture**: Pull-based iterator pattern with backpressure support
- **Event-driven integration**: Seamless integration with Arkavo's event bus

### 📝 Example Usage

```rust
// Using Claude Code capability in your agent
let capability = ClaudeCodeCapability::new(
    config,
    agent_id,
    event_writer,
    budget_tracker,
    auth_client,
).await?;

// Start a code generation run
let run_id = capability.start_run(
    "Implement a REST API endpoint for user authentication".to_string(),
    None
).await?;
```

### 🐛 Bug Fixes

- Fixed authorization client visibility issue (`get_decision` now public)
- Resolved budget tracker method compatibility
- Fixed dead code warnings with proper implementation
- Corrected event writer method naming

### 📦 Dependencies

- Added `arkavo-claude-code` v0.1.0 crate
- Runtime dependency on Node.js >= 18.0.0
- Runtime dependency on `@anthropic-ai/claude-code` npm package

### 🔄 Breaking Changes

None - this release maintains backward compatibility.

### 📚 Documentation

- New example agent in `examples/claude-code-agent/`
- Configuration guide in `AGENTS.md.example`
- Setup verification script: `check_setup.sh`

### 🙏 Acknowledgments

Special thanks to the Claude Code SDK team for providing the powerful agentic coding capabilities that make this integration possible.

---

**Full Changelog**: [v0.29.0...v0.30.0](https://github.com/arkavo-org/arkavo-edge/compare/v0.29.0...v0.30.0)

**Pull Request**: #235
**Issue**: Closes #234

🤖 Generated with [Claude Code](https://claude.ai/code)