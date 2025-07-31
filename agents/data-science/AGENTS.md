# AGENTS.md

## data-science-agent
purpose: Suggest ML models and analyze data patterns
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8352

# The data science agent specializes in:
# - ML model recommendations
# - Data analysis patterns
# - Feature engineering
# - Statistical analysis
# - Data visualization suggestions

# MCP servers for data analysis
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true