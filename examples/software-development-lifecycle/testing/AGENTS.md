# AGENTS.md

## testing-agent
purpose: Generate tests and analyze code coverage
model:   ministral-3b
listen:  0.0.0.0:8346

# The testing agent specializes in:
# - Generating unit tests
# - Creating integration tests
# - Analyzing test coverage
# - Identifying untested code paths
# - Suggesting test improvements

# MCP servers for test analysis
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