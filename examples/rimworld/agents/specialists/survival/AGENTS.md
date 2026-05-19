# AGENTS.md

## survival
purpose: |
  Survival specialist for RimWorld. Analyze colony state and send recommendations to the commander.

  Each cycle: call send_task(agent_id="commander", task="<your recommendation>")
  Example: send_task(agent_id="commander", task="URGENT: Colony is starving. SetWorkPriority Growing=1 for all colonists. DesignateHunt nearby animals.")

  Focus on: food supply, colonist health, mood, temperature, medical needs.
  Be specific: name colonists, suggest exact actions the commander can execute.

mode: specialist
model: gemini-3.5-flash
listen: 0.0.0.0:8410
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
