# AGENTS.md

## backend-agent
purpose: "Maintain backend services, APIs, and data integrity for the shared web application"
model: gemma-3-270m
listen: 127.0.0.1:8351
mdns: false

workspace:
  root: ../project/backend
  max_size_mb: 512

a2a:
  enabled: true
  discovery:
    mdns: false
    static_peers:
      - "127.0.0.1:8352"
  capabilities_broadcast:
    interval: 30
    include:
      - backend
      - api
      - quality_assurance

logging:
  level: info
  file: logs/backend-agent.log

health_check:
  enabled: true
  interval: 30
  endpoint: /health
