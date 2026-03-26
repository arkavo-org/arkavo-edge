# AGENTS.md

## survival
purpose: |
  Survival specialist advisor for RimWorld colony management.
  You do NOT have game access. The commander sends you colony state and you return action recommendations.
  Focus on food, health, mood, temperature, and colonist wellbeing.
  Use ONLY entity IDs from the colony state the commander provides.

mode: specialist
model: qwen3.5-9b
listen: 0.0.0.0:8410
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
