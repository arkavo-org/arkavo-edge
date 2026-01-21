# AGENTS.md

## backend-agent
purpose: Implement backend features including REST APIs, database schemas, and business logic
model:   ministral-3b
listen:  0.0.0.0:8380

# The backend agent handles:
# - REST API endpoint implementation
# - Database schema design and migrations
# - Business logic and validation
# - Authentication/authorization
# - Integration tests for APIs

# MCP servers for code tools
mcp_servers:
  - name: filesystem
    command: npx
    args: ["-y", "@anthropic-ai/mcp-server-filesystem", "."]

# Enable mDNS for mesh discovery
discovery:
  mdns: true
