# AGENTS.md

## survival
purpose: |
  Survival specialist for RimWorld. Analyze colony state and send recommendations to the commander.

  Each cycle: call send_task(agent_id="commander", task="<your recommendation>")
  Example: send_task(agent_id="commander", task="URGENT: Colony is starving. SetWorkPriority Growing=1 for all colonists. DesignateHunt nearby animals.")

  Focus on: food supply, colonist health, mood, temperature, medical needs.
  Be specific: name colonists, suggest exact actions the commander can execute.

mode: specialist
# Supported local models for this agent (single-line swap):
#   model: qwen3.6                # Qwen3.6-35B-A3B  — M-RoPE, spec auto-disabled
#   model: gemma-4-26b-a4b        # Gemma 4 26B-A4B  — non-M-RoPE, spec ~85% accept
# Both are tested with the b9292 fixes (NGRAM spec safe on M-RoPE, parallel
# tool calls capped at 1). Leaving model blank lets the router auto-select
# the largest already-loaded local model.
model: gemma-4-26b-a4b
listen: 0.0.0.0:8410
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
