# Claude Agent SDK Integration

<!-- ARKAVO-CAPABILITY: mcp-claude -->
> **Specs**: [9 scenarios](../../specs/arkavo-edge/mcp-claude.spec.yaml)
> **Browse**: `cargo xtask capabilities mcp-claude`
<!-- /ARKAVO-CAPABILITY -->

This example demonstrates Arkavo's native Rust integration with the Claude Agent SDK (`anthropic-agent-sdk`).

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    arkavo binary                         │
│  ┌─────────────────────────────────────────────────────┐│
│  │              ClaudeCodeCapability                    ││
│  │  ┌─────────────┐    ┌──────────────────────────┐   ││
│  │  │  SdkBridge  │───▶│  anthropic-agent-sdk     │   ││
│  │  │  (OAuth)    │    │  (native Rust crate)     │   ││
│  │  └─────────────┘    └──────────────────────────┘   ││
│  └─────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────┘
```

No Node.js required. The SDK is compiled directly into the arkavo binary.

## Authentication

The SDK supports two authentication methods:

**Option A - OAuth (Claude Max/Pro subscribers):**
```bash
# Authenticate once via Claude CLI
claude login

# Tokens are cached automatically
```

**Option B - API Key:**
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

## Usage

### Run the SDK Test

```bash
# Build arkavo
cd ../..
cargo build

# Run the SDK integration test
cargo test -p arkavo-claude-code sdk_test -- --nocapture
```

### Use in Agent Configuration

Create an `AGENTS.md` with Claude Code capability:

```yaml
name: my-coding-agent
capabilities:
  - claude_code

claude_code:
  enabled: true
  use_oauth: true  # Use OAuth if no API key set
  workspace_root: ./workspace
```

## SDK Bridge

The native integration is in `crates/arkavo-claude-code/src/sdk_bridge.rs`:

```rust
use anthropic_agent_sdk::{auth::OAuthClient, query, ClaudeAgentOptions};

// OAuth authentication
let oauth = OAuthClient::new()?;
if !oauth.is_authenticated() {
    oauth.authenticate().await?;
}

// Run a query
let stream = query(&prompt, Some(options)).await?;
while let Some(message) = stream.next().await {
    // Handle streaming response
}
```

## Files

```
crates/arkavo-claude-code/
├── src/
│   ├── sdk_bridge.rs      # Native SDK integration
│   ├── capability.rs      # Tool capability wrapper
│   ├── event_mapper.rs    # Event stream handling
│   └── config.rs          # Configuration
└── tests/
    └── sdk_test.rs        # Integration tests
```

## Learn More

- [Claude Agent SDK Docs](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/overview)
- [arkavo-claude-code crate](../../crates/arkavo-claude-code/)
