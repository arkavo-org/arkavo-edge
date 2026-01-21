# AGENTS.md

## orchestrator-agent
purpose: Coordinate frontend and backend agents to implement fullstack features
model:   ministral-3b
listen:  0.0.0.0:8390

# The orchestrator agent is responsible for:
# - Breaking down feature requests into frontend/backend tasks
# - Delegating tasks to the appropriate specialist agent
# - Coordinating API contracts between frontend and backend
# - Tracking task completion across agents
# - Aggregating results into a coherent response

# MCP servers for code tools
mcp_servers:
  - name: filesystem
    command: npx
    args: ["-y", "@anthropic-ai/mcp-server-filesystem", "."]

# Enable mDNS for mesh discovery
discovery:
  mdns: true
