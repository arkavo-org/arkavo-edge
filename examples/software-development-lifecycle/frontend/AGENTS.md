# AGENTS.md

## frontend-agent
purpose: Analyze UI/UX patterns and accessibility compliance
model:   ministral-3b
listen:  0.0.0.0:8350

# The frontend agent specializes in:
# - UI/UX best practices
# - Accessibility compliance (WCAG)
# - Responsive design patterns
# - Frontend performance optimization
# - Component architecture

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