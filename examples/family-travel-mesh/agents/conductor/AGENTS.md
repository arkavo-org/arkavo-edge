# AGENTS.md

## family-travel-conductor
purpose: "Master orchestrator for family travel planning. Decomposes objectives into subtasks and coordinates burst execution."
model: ministral-3b
listen: 0.0.0.0:8401
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8402"
    - "http://localhost:8403"
    - "http://localhost:8404"
    - "http://localhost:8410"
    - "http://localhost:8411"
    - "http://localhost:8412"
