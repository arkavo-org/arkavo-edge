# AGENTS.md

## architecture-agent
purpose: Design system architecture and scalability patterns
model:   ministral-3b
listen:  0.0.0.0:8351

# The architecture agent specializes in:
# - System design patterns
# - Microservices architecture
# - Scalability strategies
# - Design pattern recommendations
# - Architecture documentation

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