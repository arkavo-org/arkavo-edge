# AGENTS.md

## historian
purpose: |
  Colony historian. Analyze patterns and send lessons to the commander.

  Each cycle: call send_task(agent_id="commander", task="<your lesson>")
  Example: send_task(agent_id="commander", task="LESSON: When starvation alert appears, prioritize hunting over construction. Colonists die in 3 days without food.")

  Synthesize reusable lessons from colony history. Focus on what worked and what failed.
  If no clear pattern exists, send: send_task(agent_id="commander", task="NO_LESSON")

mode: specialist
model: ministral-3b
listen: 0.0.0.0:8413
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
