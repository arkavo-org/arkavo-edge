# AGENTS.md

## frontend-agent
purpose: Implement frontend features including React components, forms, and API integration
model:   ministral-3b
listen:  0.0.0.0:8370

# The frontend agent handles:
# - React/Vue component development
# - Form validation and state management
# - REST API client integration
# - CSS/styling implementation
# - Unit tests for UI components

# MCP servers for code tools
mcp_servers:
  - name: filesystem
    command: npx
    args: ["-y", "@anthropic-ai/mcp-server-filesystem", "."]

# Enable mDNS for mesh discovery
discovery:
  mdns: true
