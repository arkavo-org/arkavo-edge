# AGENTS.md

## council-synthesis
purpose: "Final synthesis coordinator for AI Council. Aggregates specialist contributions, weights by critic scores, and produces unified council response. Supports OpenTDF encryption for sensitive insights."
model: ministral-3b
listen: 0.0.0.0:8504
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8501"  # conductor
    - "http://localhost:8503"  # critic
