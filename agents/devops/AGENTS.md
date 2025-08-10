# AGENTS.md

## devops-agent
purpose: Design CI/CD pipelines and deployment strategies
model:   ollama://127.0.0.1:11434/qwen3:0.6b
listen:  0.0.0.0:8349

# The devops agent specializes in:
# - CI/CD pipeline design
# - Deployment automation
# - Infrastructure as code
# - Container orchestration
# - Monitoring and logging strategies

# MCP servers for DevOps tasks
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