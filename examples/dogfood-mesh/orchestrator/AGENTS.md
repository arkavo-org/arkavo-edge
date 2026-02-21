# AGENTS.md

## dogfood-orchestrator
purpose: |
  Orchestrate self-improvement tasks across the Arkavo Edge codebase.
  Routes tasks to specialist agents using Thompson Sampling: agents that
  produce higher-quality, compilable output get more tasks over time.

  Before each task, inject behavior guidance learned from prior failures.
  After each task, judge response quality mechanically:
  - Does the output compile? (cargo check)
  - Do tests pass? (cargo test)
  - Is it substantive? (length + specificity)

  Workflow per round:
  1. Receive a crate scan report (clippy warnings, test list, public API)
  2. Select agent via Thompson Sampling (explore vs exploit)
  3. Inject accumulated behavior guidance into the prompt
  4. Send task to selected agent
  5. Judge response quality (0.0-1.0)
  6. If quality < 0.5: extract lesson, update routing weights
  7. Gossip lessons to peers

  Categories:
  - code_review: structural analysis, clippy findings, test gap identification
  - test_generation: writing new unit tests that compile and pass

model:   glm-4.7-flash
listen:  0.0.0.0:8420

discovery:
  mdns: true
