# AGENTS.md

## architecture-agent
purpose: Design system architecture and scalability patterns
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8351

# The architecture agent specializes in:
# - System design patterns
# - Microservices architecture
# - Scalability strategies
# - Design pattern recommendations
# - Architecture documentation

# MCP servers for architecture analysis
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