# arkavo-claude-code

Claude Agent SDK integration for Arkavo Edge, providing secure and policy-controlled access to Claude's advanced coding capabilities.

## Overview

This crate integrates the Claude Agent SDK (`@anthropic-ai/claude-agent-sdk` npm package) into Arkavo Edge through a secure Node.js bridge. It provides:

- **Secure Tool Execution**: All SDK operations are subject to Arkavo's policy engine
- **Workspace Sandboxing**: File operations are confined to configured workspace roots
- **Event Streaming**: Real-time event mapping to Arkavo's event bus
- **Budget Management**: Token usage tracking and limits
- **Audit Logging**: Comprehensive logging of all tool invocations

## Architecture

The integration uses a three-layer architecture:

1. **Rust Capability Layer**: Implements the MCP tool interface and manages the lifecycle
2. **Node.js Bridge**: Runs the Claude Agent SDK in a subprocess with JSON-RPC communication
3. **Policy Layer**: Enforces security policies on all tool operations

## Configuration

Configure the capability in your `AGENTS.md` file:

### For Claude (Anthropic):
```yaml
capabilities:
  claude_code:
    enabled: true
    anthropic_model: "claude-3-sonnet-20240229"
    workspace_root: "/path/to/workspace"
    budget_tokens: 200000
```

### For DeepSeek (Anthropic-compatible):
```yaml
capabilities:
  claude_code:
    enabled: true
    anthropic_base_url: "https://api.deepseek.com/anthropic"
    anthropic_auth_token: "sk-your-deepseek-token"
    anthropic_model: "deepseek-chat"
    anthropic_small_fast_model: "deepseek-chat"
    workspace_root: "/path/to/workspace"
    budget_tokens: 200000
    
    tools:
      read: true        # Allow file read operations
      write: false      # Deny file write operations (default)
      exec: false       # Deny shell execution (default)
      web_search: false # Deny web search (default)
    
    # File access patterns
    allow_globs:
      - "src/**"
      - "tests/**"
      - "*.toml"
    
    deny_globs:  # Takes precedence over allow_globs
      - "**/.secrets/**"
      - "**/target/**"
      - "**/node_modules/**"
    
    retry:
      max_attempts: 3
      backoff_ms: 800
    
    session_ttl_secs: 3600
```

## Prerequisites

- Node.js >= 18.0.0
- npm or yarn
- `ANTHROPIC_API_KEY` environment variable

## Installation

The build script automatically installs npm dependencies when building the crate:

```bash
cargo build -p arkavo-claude-code
```

## Usage

The capability is registered as an MCP tool and can be invoked through the standard Arkavo agent interface:

```rust
use arkavo_claude_code::{ClaudeCodeCapability, ClaudeCodeConfig};

// Create capability
let capability = ClaudeCodeCapability::new(
    config,
    agent_id,
    event_writer,
    budget_tracker,
    auth_client,
).await?;

// Initialize
capability.prepare().await?;

// Open session
let session_id = capability.open_session(None).await?;

// Run task
capability.stream_run(
    "Refactor this function to use async/await".to_string(),
    session_id.clone()
).await?;

// Close session
capability.close_session(session_id).await?;
```

## Security Features

### Path Sandboxing
- All file operations are confined to the configured `workspace_root`
- Path traversal attempts are blocked
- Symlink resolution is validated

### Tool Permissions
- Default-deny policy for high-privilege operations
- Configurable per-tool permissions
- Integration with Arkavo's authorization service

### Audit Logging
- Every tool invocation is logged
- Sensitive content can be redacted
- Structured audit events for compliance

## Event Mapping

Claude Agent SDK events are mapped to Arkavo event types:

| SDK Event | Arkavo Event |
|-----------|--------------|
| `plan_updated` | `ReasoningStep` |
| `tool_use` | `ToolCall` |
| `tool_result` | `ToolResult` |
| `content_chunk` | `StreamDelta` |
| `token_usage` | `ModelResponse` (with usage) |
| `error` | `Error` |

## Testing

Run the test suite:

```bash
cargo test -p arkavo-claude-code
```

Integration tests require:
- Valid `ANTHROPIC_API_KEY`
- Node.js runtime

## Limitations

- Requires Node.js runtime (not bundled)
- npm dependencies must be installed
- Mac App Store compatibility requires bundling Node.js

## License

MIT OR Apache-2.0