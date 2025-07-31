# AGENTS.md

## testing-agent
purpose: Generate tests and analyze code coverage
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8346

# The testing agent specializes in:
# - Generating unit tests
# - Creating integration tests
# - Analyzing test coverage
# - Identifying untested code paths
# - Suggesting test improvements

# MCP servers for test analysis
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--allow-write"]
  - name: git
    command: mcp-git
    args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true