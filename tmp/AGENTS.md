# AGENTS.md — test-agent

## Agent Identity

- **Name:** test-agent
- **Mission:** "An AI agent that assists with specific tasks and capabilities"

## Runtime Configuration

```yaml
model: ollama://127.0.0.1:11434/qwen3:0.6b
listen: 0.0.0.0:8342
mdns: true
```

## Capabilities

Define what this agent can do:

- [ ] Code review and analysis
- [ ] Documentation generation
- [ ] Test creation
- [ ] Bug fixing
- [ ] Performance optimization
- [ ] Data analysis
- [ ] System design
- [ ] Security analysis

## Tool Requirements

Specify which tools this agent needs:

- [ ] Git tools (version control operations)
- [ ] Filesystem access (read/write files)
- [ ] Terminal/Shell (execute commands)
- [ ] Code analysis tools
- [ ] Database access
- [ ] Web APIs
- [ ] Docker/Container management

## MCP Servers

Configure MCP servers that this agent should use:

```yaml
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: []
  - name: git
    command: mcp-git
    args: []
```

## Agent Configuration

Customize the following values for your agent:

```yaml
purpose: "Describe your agent's primary purpose and goals here"
model: ollama://127.0.0.1:11434/qwen3:0.6b
listen: 0.0.0.0:8342
mdns: true
```

## API Keys (Optional)

If your agent needs to access external services, add API keys here:

```yaml
# OPENAI_API_KEY: sk-xxx
# MOONSHOT_API_KEY: sk-xxx
# DEEPSEEK_API_KEY: sk-xxx
```

## Notes

1. Edit the **Mission** field to describe what your agent does
2. Check the capabilities your agent should have
3. Select the tools your agent needs access to
4. Configure MCP servers for additional functionality
5. Update the model if you want to use a different LLM
6. Change the listen address if needed (default: 0.0.0.0:8342)
7. Set mdns to false if you don't want network discovery

## Running Your Agent

Once configured, run your agent with:

```bash
arkavo agent run
```

Your agent will start and be available at the configured address.