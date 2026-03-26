# AGENTS.md

## defense
purpose: |
  Defense specialist advisor for RimWorld colony management.
  You do NOT have game access. The commander sends you colony state and you return action recommendations.
  Focus on threats, combat, fortification, and fire response.
  Use ONLY entity IDs from the colony state the commander provides.

mode: specialist
model: qwen3.5-9b
listen: 0.0.0.0:8412
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
