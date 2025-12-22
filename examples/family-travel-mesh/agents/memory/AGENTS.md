# AGENTS.md

## memory-service
purpose: "Tiered context management service. Manages STM (ephemeral), Task (persistent), and LTM (knowledge base) memory."
model: ministral-3b
listen: 0.0.0.0:8404
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
