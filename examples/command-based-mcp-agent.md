# Example: Command-Based MCP Servers

This example demonstrates how to configure an agent with command-based MCP servers.

## Setup

1. Create an `AGENTS.md` file:

```markdown
## my-agent
purpose: AI agent with filesystem and git access
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8342

mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--allow-write", "/tmp"]
  - name: git
    command: mcp-git
    args: ["--read-only"]
  - name: external-server
    url: http://localhost:8080
```

2. Run the agent:

```bash
arkavo agent run
```

## What happens

1. The agent starts an A2A server on port 8342
2. It spawns the `mcp-filesystem` command with write access to `/tmp`
3. It spawns the `mcp-git` command in read-only mode
4. It connects to an external MCP server at `http://localhost:8080`
5. All MCP servers are registered and their tools become available to the agent
6. When the agent shuts down (Ctrl+C), all spawned processes are terminated

## Command validation

If a command doesn't exist, you'll see an error:

```
Failed to spawn MCP server filesystem (mcp-filesystem): Command 'mcp-filesystem' not found in PATH. Please ensure it is installed and accessible.
```

## Process management

- Each spawned MCP server runs as a child process
- Process IDs are tracked for cleanup
- On shutdown, processes receive SIGTERM first, then SIGKILL if needed
- stderr output from MCP servers is logged for debugging