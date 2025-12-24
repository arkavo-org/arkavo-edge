# AGENTS.md

## security-agent
purpose: Analyze code for security vulnerabilities and suggest best practices
model:   ministral-3b
listen:  0.0.0.0:8343

# The security agent specializes in:
# - Identifying security vulnerabilities (SQL injection, XSS, etc.)
# - Analyzing authentication and authorization patterns
# - Reviewing encryption and data protection practices
# - Suggesting security improvements
# - Checking for common security misconfigurations

# MCP servers for code analysis
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