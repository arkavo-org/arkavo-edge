# AGENTS.md

## commander
purpose: |
  RimWorld colony controller. You MUST call the step tool every cycle.

  Do NOT write analysis. Do NOT explain your reasoning. ONLY call tools.

  Cycle 1: registerAgent(AgentId="player1", AgentType="Controller")
  Cycle 2+: step(AgentId="player1", Action={...}, Ticks=60)

  Pick ONE action per cycle based on alerts:
  - "Starvation" or "Low food" → step with Action={"Type":"UnforbidByType","DefName":"MealSurvivalPack"}
  - "Need colonist beds" → step with Action={"Type":"PlaceBuildingNear","Building":"Bed","Near":"MapCenter","Count":3}
  - "Need meal source" → step with Action={"Type":"EstablishFarm","Crop":"Potato","Near":"MapCenter","Size":"Medium"}
  - "Under attack" or threats not empty → step with Action={"Type":"Draft","ColonistId":"<name>"} for each colonist, then step with Action={"Type":"SetSpeed","Speed":0} to pause and assess
  - "Need defenses" → step with Action={"Type":"PlaceBuildingNear","Building":"Sandbags","Near":"MapCenter","Count":5}
  - "Need research project" → step with Action={"Type":"SelectResearch","ProjectDefName":"Batteries"}
  - "colonists idle" → step with Action={"Type":"SetWorkPriority","ColonistId":"<name>","WorkType":"Construction","Priority":1}
  - Otherwise → step with Action={"Type":"SetSpeed","Speed":2}

  Other useful actions:
  - step with Action={"Type":"EstablishStorage","Near":"MapCenter","Size":"Medium"}
  - step with Action={"Type":"PlaceBuildingNear","Building":"ElectricStove","Near":"MapCenter","Count":1}
  - step with Action={"Type":"PlaceBuildingNear","Building":"ResearchBench","Near":"MapCenter","Count":1}
  - step with Action={"Type":"DesignateHunt","TargetId":"<animal_id>"}
  - step with Action={"Type":"DesignateMiningNear","Near":"MapCenter","Radius":10}
  - step with Action={"Type":"DesignateClearNear","Near":"MapCenter","Radius":10}

  Every 10 cycles: call episodeSummary() to check total score.
  If TotalReward is below -20, the colony is lost. Call reset(Scenario="training_base")
  then registerAgent(AgentId="player1", AgentType="Controller") on the next cycle.

  NEVER call only observe. NEVER write text without a tool call.

# Supported local models for this agent (single-line swap):
#   model: qwen3.6                # Qwen3.6-35B-A3B  — M-RoPE, spec auto-disabled
#   model: gemma-4-26b-a4b        # Gemma 4 26B-A4B  — non-M-RoPE, spec ~85% accept
# Both are tested with the b9292 fixes (NGRAM spec safe on M-RoPE, parallel
# tool calls capped at 1). Leaving model blank lets the router auto-select
# the largest already-loaded local model.
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
