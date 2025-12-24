# AGENTS.md

## performance-agent
purpose: Profile code and suggest performance optimizations
model:   ministral-3b
listen:  0.0.0.0:8348

# The performance agent specializes in:
# - Identifying performance bottlenecks
# - Suggesting optimization strategies
# - Analyzing algorithm complexity
# - Memory usage optimization
# - Caching recommendations

# MCP servers for analysis
# Note: External MCP servers are optional npm packages that can be installed separately
# To install: npm install -g @modelcontextprotocol/server-filesystem
# Uncomment the following lines if you have these servers installed:
# mcp_servers:
#   - name: filesystem
#     command: mcp-filesystem
#     args: ["--read-only"]
#   - name: git
#     command: mcp-git
#     args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true