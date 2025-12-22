# AGENTS.md

## family-safety-critic
purpose: "Quality assurance gate. Evaluates specialist outputs against family safety policies before approval."
model: ministral-3b
listen: 0.0.0.0:8403
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8402"
