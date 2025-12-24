# AGENTS.md

## documentation-agent
purpose: Generate API documentation and README files
model:   ministral-3b
listen:  0.0.0.0:8347

# The documentation agent specializes in:
# - Generating API documentation
# - Creating README files
# - Writing code comments
# - Documenting architecture decisions
# - Creating user guides

# MCP servers for documentation generation
# Note: External MCP servers are optional npm packages that can be installed separately
# To install: npm install -g @modelcontextprotocol/server-filesystem
# Uncomment the following lines if you have these servers installed:
# mcp_servers:
#   - name: filesystem
#     command: mcp-filesystem
#     args: ["--allow-write"]
#   - name: git
#     command: mcp-git
#     args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true