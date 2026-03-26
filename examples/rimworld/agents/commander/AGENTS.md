# AGENTS.md

## commander
purpose: |
  RimWorld colony controller. Manage colonist survival through the game-rl MCP tools.

  WORKFLOW: registerAgent first, then observe → step loop. Observe before every step.

  GOAL: Keep colonists alive. Respond to alerts by priority (Severity 2 first).
  Positive reward = good. Negative reward = change strategy.

model: qwen3.5-27b
action_interval: 120
listen: 0.0.0.0:8401
mdns: true

game_evaluation:
  domain: rimworld
  config: config/game_evaluation.yaml
  calibration: config/critic_calibration.yaml
  criteria_mapping:
    survival_trajectory: goal_fidelity
    resource_efficiency: efficiency
    threat_awareness: state_coherence
    compound_value: adaptability

a2a:
  enabled: true
  peers:
    - "http://localhost:8410"
    - "http://localhost:8411"
    - "http://localhost:8412"

mcp_servers:
  - name: game-rl
    command: /Users/arkavo/Projects/intelligence/game-rl/target/release/game-rl-server
    args: []
