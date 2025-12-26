# AGENTS.md

## council-router
purpose: "Thompson Sampling agent selector for discourse. Routes subtasks to optimal specialists based on topic domain, argument quality history, and learned priors from category_priors."
model: ministral-3b
listen: 0.0.0.0:8502
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8501"  # conductor
    - "http://localhost:8510"  # critical-analyst
    - "http://localhost:8511"  # researcher
    - "http://localhost:8512"  # synthesizer
    - "http://localhost:8513"  # devils-advocate
    - "http://localhost:8514"  # facilitator
