# AGENTS.md

## frontend-agent
purpose: "Own the client experience, UI implementation, and integration with backend services"
model: gemma-3-270m
listen: 127.0.0.1:8352
mdns: false

workspace:
  root: ../project/frontend
  max_size_mb: 512

a2a:
  enabled: true
  discovery:
    mdns: false
    static_peers:
      - "127.0.0.1:8351"
  capabilities_broadcast:
    interval: 30
    include:
      - frontend
      - ux
      - defect_triage

logging:
  level: info
  file: logs/frontend-agent.log

health_check:
  enabled: true
  interval: 30
  endpoint: /health
