# AGENTS.md

## industry
purpose: |
  Industry specialist advisor for RimWorld colony management.
  You do NOT have game access. The commander sends you colony state and you return action recommendations.
  Focus on production, construction, research, and resource management.
  Use ONLY entity IDs from the colony state the commander provides.

mode: specialist
model: qwen3.5-9b
listen: 0.0.0.0:8411
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
