# AGENTS.md

## survival
purpose: |
  Survival specialist advisor for RimWorld colony management.
  You do NOT have game access. Use send_task to ask the commander for colony state, then analyze and send back recommendations.
  Focus on food, health, mood, temperature, and colonist wellbeing.

mode: specialist
model: qwen3.5-9b
listen: 0.0.0.0:8410
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
