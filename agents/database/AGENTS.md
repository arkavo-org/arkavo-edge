# AGENTS.md

## database-agent
purpose: Optimize SQL queries and design database schemas
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8345

# The database agent specializes in:
# - SQL query optimization
# - Database schema design
# - Index recommendations
# - Data modeling best practices
# - Database performance tuning

# MCP servers for database analysis
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true