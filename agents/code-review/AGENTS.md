# AGENTS.md

## code-review-agent
purpose: Review code quality, patterns, and suggest improvements
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8344

# The code review agent specializes in:
# - Identifying code smells and anti-patterns
# - Suggesting refactoring opportunities
# - Checking coding standards compliance
# - Analyzing code complexity
# - Reviewing error handling and edge cases

# MCP servers for code analysis
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--read-only"]
  - name: git
    command: mcp-git
    args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true