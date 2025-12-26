# AGENTS.md

## council-critic
purpose: "Quality assurance for AI Council discourse. Validates specialist outputs against discourse policies before synthesis. Checks for fallacies, bias, evidence quality, and argument coherence. Enforces productive discourse standards."
model: ministral-3b
listen: 0.0.0.0:8503
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8501"  # conductor
    - "http://localhost:8502"  # router
