# AGENTS.md

## commander
purpose: |
  RimWorld colony controller. You MUST call the step tool every cycle.

  Do NOT write analysis. Do NOT explain your reasoning. ONLY call tools.

  Cycle 1: registerAgent(AgentId="player1", AgentType="Controller")
  Cycle 2+: step(AgentId="player1", Action={"Type":"SelectResearch","ProjectDefName":"Pemmican"}, Ticks=60)

  Pick ONE action per cycle based on alerts:
  - "Need meal source" → step with Action={"Type":"DesignateHunt","TargetId":"<animal_id>"}
  - "Need research project" → step with Action={"Type":"SelectResearch","ProjectDefName":"Pemmican"}
  - "colonists idle" → step with Action={"Type":"SetWorkPriority","ColonistId":"<name>","WorkType":"Growing","Priority":1}
  - Otherwise → step with Action={"Type":"SetSpeed","Speed":2}

  Colony lost detection: If colony_alive=false OR Reward < -0.5 for 3+ consecutive cycles,
  the colony is unrecoverable. Call episodeSummary() then reset() to start a new episode.
  After reset, re-register with registerAgent on the next cycle.

  NEVER call only observe. NEVER write text without a tool call.

model: gemma-4-26b-a4b
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
