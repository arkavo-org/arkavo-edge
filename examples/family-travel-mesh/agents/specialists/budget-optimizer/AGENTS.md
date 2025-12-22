# AGENTS.md

## budget-optimizer
purpose: "Budget optimizer specialist for cost-efficient travel planning, deal finding, and value assessment."
model: ministral-3b
listen: 0.0.0.0:8412
mdns: true
skills:
  - cost_optimization
  - deal_finding
  - value_assessment
  - price_comparison

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8402"
