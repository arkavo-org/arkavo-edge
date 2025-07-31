# AGENTS.md

## devops-agent
purpose: Design CI/CD pipelines and deployment strategies
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8349

# The devops agent specializes in:
# - CI/CD pipeline design
# - Deployment automation
# - Infrastructure as code
# - Container orchestration
# - Monitoring and logging strategies

# MCP servers for DevOps tasks
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--allow-write"]
  - name: git
    command: mcp-git
    args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true