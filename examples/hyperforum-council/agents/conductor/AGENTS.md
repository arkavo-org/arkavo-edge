# AGENTS.md

## council-conductor
purpose: "Master orchestrator for HYPERforum AI Council deliberation. Decomposes discourse questions into analysis subtasks, coordinates specialist bursts, and manages synthesis workflow. Enforces debate structure and ensures all perspectives are represented."
model: ministral-3b
listen: 0.0.0.0:8501
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8502"  # router
    - "http://localhost:8503"  # critic
    - "http://localhost:8504"  # synthesis
    - "http://localhost:8510"  # critical-analyst
    - "http://localhost:8511"  # researcher
    - "http://localhost:8512"  # synthesizer
    - "http://localhost:8513"  # devils-advocate
    - "http://localhost:8514"  # facilitator
