# Claude Agent SDK Integration

<!-- ARKAVO-CAPABILITY: mcp-claude -->
> **Specs**: [9 scenarios](../../specs/arkavo-edge/mcp-claude.spec.yaml)
> **Browse**: `cargo xtask capabilities mcp-claude`
<!-- /ARKAVO-CAPABILITY -->

Native Rust integration with the Claude Agent SDK using bidirectional
`ClaudeSDKClient` sessions, budget tracking, and MCP tool registration.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                       arkavo binary                           │
│  ┌────────────────────────────────────────────────────────┐  │
│  │              ClaudeCodeCapability                       │  │
│  │  ┌──────────────┐    ┌────────────────────────────┐   │  │
│  │  │  SdkBridge   │───▶│  anthropic-agent-sdk       │   │  │
│  │  │  (bidir/1shot)│    │  (ClaudeSDKClient)        │   │  │
│  │  └──────┬───────┘    └────────────────────────────┘   │  │
│  │         │                                              │  │
│  │  ┌──────▼───────┐    ┌────────────────────────────┐   │  │
│  │  │ HookHandler  │───▶│  EventMapper (AG-UI)       │   │  │
│  │  │ Permissions  │    │  Budget · Metrics · Tools   │   │  │
│  │  └──────────────┘    └────────────────────────────┘   │  │
│  │                                                        │  │
│  │  MCP Tools: claude_code_run · plan · session_info ·   │  │
│  │             interrupt                                  │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

No Node.js required. The SDK is compiled directly into the arkavo binary.

## Authentication

**Option A - OAuth (Claude Max/Pro subscribers):**
```bash
claude login
```

**Option B - API Key:**
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

## Quick Start

```bash
# From repo root
cargo build -q

# Run a coding task with Claude Code tools
cargo run -p arkavo -- chat --prompt "Explain the Fibonacci function"

# Run with debug logging
ARKAVO_DEBUG=1 cargo run -p arkavo -- chat --prompt "Write a unit test for email validation"
```

## MCP Tools

Four tools are registered when Claude Code auth is available:

| Tool | Aliases | Purpose |
|------|---------|---------|
| `claude_code_run` | `cc_run` | Execute a coding task with full SDK capabilities |
| `claude_code_plan` | `cc_plan` | Generate a plan without execution |
| `claude_code_session_info` | `cc_info` | Get model, tools, budget status |
| `claude_code_interrupt` | `cc_interrupt` | Interrupt a running task |

## Budget Tracking

`compute_budget_status()` returns AG-UI compatible metrics:
```json
{
  "total_cost_usd": 0.023,
  "max_budget_usd": 5.0,
  "remaining_cost_usd": 4.977,
  "used_input_tokens": 1200,
  "used_output_tokens": 450,
  "used_tokens": 1650
}
```

## Configuration

See `AGENTS.md` for the full agent configuration including:
- Bidirectional session mode (`use_bidirectional: true`)
- Budget limits (`max_budget_usd`, `budget_tokens`)
- Tool permissions and filtering (`allowed_tools`, `disallowed_tools`)
- File access patterns (`allow_globs`, `deny_globs`)
- Permission mode (`default`, `plan`, `acceptEdits`)

## Files

```
crates/arkavo-mcp-claude/
├── src/
│   ├── sdk_bridge.rs      # Bidirectional + one-shot SDK integration
│   ├── capability.rs      # ClaudeCodeCapability with budget tracking
│   ├── hook_handler.rs    # SDK hooks + permission callbacks
│   ├── event_mapper.rs    # AG-UI event emission
│   ├── policy_bridge.rs   # Authorization policy enforcement
│   ├── config.rs          # Configuration with SDK options builder
│   └── tools/             # MCP tool implementations
└── tests/
    ├── basic_test.rs            # Config + capability unit tests
    ├── bidirectional_test.rs    # SDK session integration tests
    └── sdk_test.rs              # Raw SDK integration tests
```
