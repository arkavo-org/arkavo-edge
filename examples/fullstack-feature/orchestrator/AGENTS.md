# AGENTS.md

## orchestrator-agent
purpose: Coordinate frontend and backend agents to implement fullstack features
listen:  0.0.0.0:8362

# The orchestrator agent is responsible for:
# - Breaking down feature requests into frontend/backend tasks
# - Delegating tasks to the appropriate specialist agent
# - Coordinating API contracts between frontend and backend
# - Tracking task completion across agents
# - Aggregating results into a coherent response

# Enable mDNS for mesh discovery
discovery:
  mdns: true
