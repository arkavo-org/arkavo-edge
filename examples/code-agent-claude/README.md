# Claude Code Agent Example

This example demonstrates an Arkavo agent using the native Rust Claude Agent SDK to perform sophisticated coding tasks with full file system access and code generation capabilities.

## Overview

The Claude Code Agent showcases:
- Native Rust integration with `anthropic-agent-sdk` (no Node.js required)
- OAuth authentication for Claude Max/Pro subscribers
- API key authentication as fallback
- Policy-controlled file operations and code generation
- Budget tracking for API usage
- Event streaming for real-time progress updates

## Prerequisites

### 1. Authentication (Choose One)

**Option A - OAuth (Claude Max/Pro subscribers):**
```bash
# Login via Claude CLI (recommended)
claude login

# Or the agent will prompt for browser authentication on first use
```

**Option B - API Key:**
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### 2. Build Arkavo

```bash
cd ../..
cargo build
```

## Quick Start

### 1. Start the Claude Code Agent

```bash
./launch_agent.sh
```

This starts an agent on port 8345 with Claude Code capability enabled.

### 2. Run Example Tasks

```bash
# Simple code generation
./run_examples.sh generate

# Code analysis and improvement
./run_examples.sh analyze

# Full project scaffolding
./run_examples.sh scaffold

# Interactive coding session
./run_examples.sh interactive
```

### 3. Monitor Progress

```bash
# View agent logs
tail -f logs/claude-code-agent.log

# Or use the AGUI dashboard
arkavo ui
# Open http://localhost:3000
```

## Example Tasks

### 1. Generate a REST API

```bash
curl -X POST http://localhost:8345/v1/agent/task \
  -H "Content-Type: application/json" \
  -d '{
    "task": "Create a REST API for a todo list application",
    "tool": "claude_code_run",
    "workspace": "./workspace"
  }'
```

### 2. Analyze and Improve Code

```bash
curl -X POST http://localhost:8345/v1/agent/task \
  -H "Content-Type: application/json" \
  -d '{
    "task": "Analyze the code in workspace/ and suggest improvements",
    "tool": "claude_code_plan",
    "workspace": "./workspace"
  }'
```

### 3. Generate Tests

```bash
curl -X POST http://localhost:8345/v1/agent/task \
  -H "Content-Type: application/json" \
  -d '{
    "task": "Generate comprehensive tests for the TodoAPI class",
    "tool": "claude_code_run",
    "workspace": "./workspace"
  }'
```

## Agent Configuration

The agent is configured via `AGENTS.md`:

```yaml
name: claude-code-agent
port: 8345
model: claude-3-sonnet-20240229  # Or deepseek-chat for DeepSeek

capabilities:
  - claude_code_run    # Execute coding tasks
  - claude_code_plan   # Generate plans without execution

claude_code:
  enabled: true
  workspace_root: ./workspace
  budget_tokens: 200000
  tools:
    read: true
    write: true
    exec: false      # Disabled for safety
    web_search: true
  allow_globs:
    - "**/*.js"
    - "**/*.ts"
    - "**/*.py"
    - "**/*.rs"
  deny_globs:
    - "**/.env"
    - "**/secrets/**"
```

## Security and Policy

The Claude Code capability includes several security features:

### 1. Workspace Sandboxing
- All file operations are restricted to the configured workspace
- Path traversal attempts are blocked
- Symlink resolution is validated

### 2. Tool Permissions
- Fine-grained control over read/write/exec/web operations
- Glob patterns for allowed/denied file paths
- Authorization service integration for advanced policies

### 3. Budget Management
- Token usage tracking
- Cost estimation and limits
- Automatic throttling when approaching limits

## Monitoring and Events

The agent emits various events that can be monitored:

### Event Types
- `SessionStarted` - Claude Code session initialized
- `PromptSent` - Task prompt sent to Claude
- `ToolCall` - File operation or other tool invoked
- `ToolResult` - Result of tool execution
- `StreamDelta` - Real-time content generation
- `ModelResponse` - Final response from Claude
- `SessionEnded` - Session cleanup

### View Events

```bash
# Via logs
tail -f logs/claude-code-agent.log | grep EVENT

# Via WebSocket
wscat -c ws://localhost:8345/ws
```

## Advanced Usage

### Custom Workspace

```bash
# Create custom workspace
mkdir -p /tmp/claude-workspace

# Update AGENTS.md
sed -i 's|./workspace|/tmp/claude-workspace|g' AGENTS.md

# Restart agent
./launch.sh restart
```

### Rate Limiting

Configure rate limits in AGENTS.md:

```yaml
claude_code:
  rate_limit:
    max_attempts: 3
    backoff_ms: 800
  session_ttl: 3600
```

## Troubleshooting

### OAuth Authentication Issues

```bash
# Check if already authenticated
# The agent checks for cached tokens automatically

# Re-authenticate via Claude CLI
claude login

# Or use API key instead
export ANTHROPIC_API_KEY="sk-ant-..."
```

### API Key Issues

```bash
# Verify API key is set
echo $ANTHROPIC_API_KEY

# Test API directly
curl https://api.anthropic.com/v1/messages \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "anthropic-version: 2023-06-01"
```

### Permission Denied

```bash
# Ensure workspace is writable
chmod -R 755 workspace/

# Check file permissions in logs
grep "PolicyViolation" logs/claude-code-agent.log
```

## Examples Directory Structure

```
claude-code-agent/
├── AGENTS.md              # Agent configuration
├── workspace/             # Working directory for code
│   └── .gitkeep
├── logs/                  # Agent logs
│   └── .gitkeep
├── launch_agent.sh        # Start/stop script
├── run_examples.sh        # Example task runner
├── test_connection.sh     # Test Claude Code SDK
└── README.md             # This file
```

## Integration with Other Agents

The Claude Code agent can work with other Arkavo agents:

```bash
# Start multiple agents
cd ../software-development-simple
./launch_agents.sh

cd ../claude-code-agent
./launch_agent.sh

# Project Manager can delegate to Claude Code agent
curl -X POST http://localhost:8342/v1/agent/message \
  -d '{
    "to_agent": "claude-code-agent",
    "task": "Implement the Calculator class with add, subtract, multiply, divide methods"
  }'
```

## Performance Tips

1. **Use DeepSeek for development** - Lower cost, good for iteration
2. **Enable caching** - Reduces redundant API calls
3. **Set appropriate timeouts** - Prevent hanging on long tasks
4. **Use plan mode first** - Get a plan before execution
5. **Monitor token usage** - Track costs via events

## Learn More

- [Claude Agent SDK Documentation](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/overview)
- [Arkavo Claude Code Integration](../../crates/arkavo-claude-code/)
- [Arkavo Documentation](../../README.md)

## License

This example is part of the Arkavo project and follows the same license terms.