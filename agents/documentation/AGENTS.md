# AGENTS.md

## documentation-agent
purpose: Generate API documentation and README files
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8347

# The documentation agent specializes in:
# - Generating API documentation
# - Creating README files
# - Writing code comments
# - Documenting architecture decisions
# - Creating user guides

# MCP servers for documentation generation
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