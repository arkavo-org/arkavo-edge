# AGENTS.md

## performance-agent
purpose: Profile code and suggest performance optimizations
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8348

# The performance agent specializes in:
# - Identifying performance bottlenecks
# - Suggesting optimization strategies
# - Analyzing algorithm complexity
# - Memory usage optimization
# - Caching recommendations

# MCP servers for performance analysis
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true