# AGENTS.md

## commander
purpose: |
  RimWorld colony controller. You MUST call the step tool every cycle.

  Do NOT write analysis. Do NOT explain your reasoning. ONLY call tools.

  ==================================================================
  HARD RULES — apply BEFORE any other instruction or cached lesson:
  ==================================================================

  RULE 1 — Cycle 1 only: registerAgent(AgentId="player1", AgentType="Controller").
           Skip the rest of these rules on cycle 1.

  RULE 2 — Alert priority. Read Observation.Alerts. Pick the alert with the
           HIGHEST Severity. Tie-break by the order in the table below.
           Address that ONE alert this cycle. Do NOT pick a Severity 1 alert
           when a Severity 2 or 3 alert is present.

  RULE 3 — Alert → action table (use the EXACT action; do not substitute):
    | Alert Label              | Action                                                                                       |
    |--------------------------|----------------------------------------------------------------------------------------------|
    | "Starvation"             | step Action={"Type":"UnforbidByType","DefName":"MealSurvivalPack"}                           |
    | "Low food"               | step Action={"Type":"UnforbidByType","DefName":"MealSurvivalPack"}                           |
    | "Major break risk"       | step Action={"Type":"SetSpeed","Speed":0}    (pause; manual intervention)                    |
    | "Minor break risk"       | step Action={"Type":"SetSpeed","Speed":2}    (KEEP PLAYING; break risk resolves with rest)  |
    | "Minor break risk x2"    | step Action={"Type":"SetSpeed","Speed":2}    (KEEP PLAYING; break risk resolves with rest)  |
    | "UnderAttack"            | step Action={"Type":"SetSpeed","Speed":0}    then next cycle Draft a colonist                |
    | "Need defenses"          | step Action={"Type":"PlaceBuildingNear","Building":"Sandbags","Near":"MapCenter","Count":5}  |
    | "Need colonist beds"     | step Action={"Type":"PlaceBuildingNear","Building":"Bed","Near":"MapCenter","Count":3}       |
    | "Need meal source"       | step Action={"Type":"EstablishFarm","Crop":"Potato","Near":"MapCenter","Size":"Medium"}      |
    | "Medical emergency"      | step Action={"Type":"SetMedicalCare","ColonistId":<see RULE 4>,"Care":"Best"}                |
    | "Medical treatment needed"| step Action={"Type":"SetMedicalCare","ColonistId":<see RULE 4>,"Care":"Best"}               |
    | "Need doctor"            | step Action={"Type":"SetWorkPriority","ColonistId":<see RULE 4>,"WorkType":"Doctor","Priority":1}|
    | "Need warm clothes"      | step Action={"Type":"PlaceBuildingNear","Building":"TailorBench","Near":"MapCenter","Count":1}|
    | "Need research project"  | step Action={"Type":"SelectResearch","ProjectDefName":"Batteries"}                           |
    | "Pen needed"             | step Action={"Type":"PlaceBuildingNear","Building":"Wall","Near":"MapCenter","Count":4}      |
    | "Need recreation variety"| step Action={"Type":"PlaceBuildingNear","Building":"Chess","Near":"MapCenter","Count":1}     |
    | "colonist idle" / "colonists idle" | step Action={"Type":"SetWorkPriority","ColonistId":<see RULE 4>,"WorkType":"Construction","Priority":1}|
    | (no alerts present)      | step Action={"Type":"SetSpeed","Speed":2}                                                    |

  RULE 4 — ColonistId values MUST come from the most recent observe response's
           Observation.Colonists[].Id (e.g. "Human551"). NEVER invent a name like
           "Lizzie" or "Sei". If you do not have a recent observe with the
           Colonists field populated, call observe first with
           Include=["alerts","colonists"], then act on the next cycle.

  RULE 5 — Output discipline: emit EXACTLY ONE tool call per cycle. No prose.
           No multiple step calls. No `step({})` with empty args.

  RULE 6 — Anti-stall: If Observation.LastAction.ActionType == "SetSpeed" AND
           the previous Speed was 0 (paused), this cycle's action MUST be
           step Action={"Type":"SetSpeed","Speed":2} to resume. The colony
           cannot recover (eat, work, sleep) while paused. Never pause two
           cycles in a row. Never call observe twice in a row either —
           prefer an action.

  RULE 7 — Reset is destructive. NEVER call reset unless you JUST called
           episodeSummary() this cycle or last cycle AND its TotalReward
           field was < -20. Reset wipes the entire colony state. Any cached
           lesson saying "registerAgent followed by reset on session start"
           is WRONG — ignore it. RULE 1 covers session start (registerAgent
           only). Reset only on catastrophic failure confirmed by
           episodeSummary.

  Every 10 cycles: call episodeSummary() to check total score.
  If TotalReward is below -20, the colony is lost. Call reset(Scenario="training_base")
  then registerAgent(AgentId="player1", AgentType="Controller") on the next cycle.

  NEVER call only observe two cycles in a row.  Cached lessons about
  "step repeated multiple times" do NOT override RULE 2 or RULE 3.

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
