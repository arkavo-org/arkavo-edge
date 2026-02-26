# AGENTS.md

## vision-orchestrator
purpose: |
  Orchestrate vision analysis tasks across a mesh of specialist agents.
  Routes image analysis requests to the vision-analyst agent and
  aggregates results for multi-image workflows.

  Supported task types:
  - UI screenshot analysis (layout, accessibility, component identification)
  - Architecture diagram interpretation
  - Chart and graph data extraction
  - General image description and comparison

model:   qwen3.5-27b
listen:  0.0.0.0:8418

discovery:
  mdns: true
