# AGENTS.md

## agent-router
purpose: "Thompson Sampling agent selector. Routes subtasks to optimal specialists based on learned priors."
model: ministral-3b
listen: 0.0.0.0:8402
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8410"
    - "http://localhost:8411"
    - "http://localhost:8412"
