# AGENTS.md

## frontend-agent
purpose: Analyze UI/UX patterns and accessibility compliance
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8350

# The frontend agent specializes in:
# - UI/UX best practices
# - Accessibility compliance (WCAG)
# - Responsive design patterns
# - Frontend performance optimization
# - Component architecture

# MCP servers for frontend analysis
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true