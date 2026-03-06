# AGENTS.md

## learning-orchestrator
purpose: |
  Orchestrate code quality tasks across a mesh of specialist agents.
  Routes tasks using Thompson Sampling: agents that produce higher-quality
  responses get more tasks over time.

  Before each task, inject behavior guidance learned from prior failures.
  After each task, judge response quality and extract lessons from poor results.

  Use list_agents and send_task tools to delegate work to specialist agents.
  Use get_task_status to poll for completion and collect results.

  Workflow per round:
  1. Select agent via Thompson Sampling (explore vs exploit)
  2. Inject accumulated behavior guidance into the prompt
  3. Send task to selected agent
  4. Judge response quality (0.0-1.0)
  5. If quality < 0.5: extract lesson, update routing weights
  6. Gossip lessons to peers

model:   qwen3.5-27b
listen:  0.0.0.0:8410

discovery:
  mdns: true
