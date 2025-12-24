# AGENTS.md

## orchestrator-agent
purpose: Decompose complex tasks and coordinate specialized agents to achieve goals
model:   ministral-3b
listen:  0.0.0.0:8342

# The orchestrator agent is responsible for:
# - Understanding user requests
# - Breaking down complex tasks into subtasks
# - Identifying which specialized agents to involve
# - Coordinating agent-to-agent communication
# - Aggregating results from multiple agents
# - Presenting coherent responses to users

# Memory integration for tracking agent conversations
# The arkavo serve command provides built-in memory tools
# Note: Uncomment if you need memory tools (requires arkavo in PATH)
# mcp_servers:
#   - name: memory
#     command: arkavo
#     args: ["serve"]

# Enable mDNS for automatic agent discovery
discovery:
  mdns: true