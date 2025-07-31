# AGENTS.md

## orchestrator-agent
purpose: Decompose complex tasks and coordinate specialized agents to achieve goals
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8342

# The orchestrator agent is responsible for:
# - Understanding user requests
# - Breaking down complex tasks into subtasks
# - Identifying which specialized agents to involve
# - Coordinating agent-to-agent communication
# - Aggregating results from multiple agents
# - Presenting coherent responses to users

# Memory integration for tracking agent conversations
mcp_servers:
  - name: memory
    command: arkavo-memory-server
    args: ["--port", "8001"]

# Enable mDNS for automatic agent discovery
discovery:
  mdns: true