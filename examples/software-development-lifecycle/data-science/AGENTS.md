# AGENTS.md

## data-science-agent
purpose: Suggest ML models and analyze data patterns
model:   ministral-3b
listen:  0.0.0.0:8352

# The data science agent specializes in:
# - ML model recommendations
# - Data analysis patterns
# - Feature engineering
# - Statistical analysis
# - Data visualization suggestions

# MCP servers for data analysis
# Note: External MCP servers are optional npm packages that can be installed separately
# To install: npm install -g @modelcontextprotocol/server-filesystem
# Uncomment the following lines if you have these servers installed:
# mcp_servers:
#   - name: filesystem
#     command: mcp-filesystem
#     args: ["--read-only"]

# Enable mDNS for automatic discovery
discovery:
  mdns: true