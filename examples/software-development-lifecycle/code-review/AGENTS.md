# AGENTS.md

## code-review-agent
purpose: Review code quality, patterns, and suggest improvements
model:   ministral-3b
listen:  0.0.0.0:8344
swarm:   code-review

# The code review agent specializes in:
# - Identifying code smells and anti-patterns
# - Suggesting refactoring opportunities
# - Checking coding standards compliance
# - Analyzing code complexity
# - Reviewing error handling and edge cases

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