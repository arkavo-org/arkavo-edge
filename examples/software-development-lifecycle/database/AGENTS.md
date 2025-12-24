# AGENTS.md

## database-agent
purpose: Optimize SQL queries and design database schemas
model:   ministral-3b
listen:  0.0.0.0:8345

# The database agent specializes in:
# - SQL query optimization
# - Database schema design
# - Index recommendations
# - Data modeling best practices
# - Database performance tuning

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