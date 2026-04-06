# AGENTS.md

## defense
purpose: |
  Defense specialist advisor for RimWorld colony management.
  You do NOT have game access. Use send_task to ask the commander for colony state, then analyze and send back recommendations.
  Focus on threats, combat, fortification, and fire response.

mode: specialist
model: gemma-4-26b-a4b
listen: 0.0.0.0:8412
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
