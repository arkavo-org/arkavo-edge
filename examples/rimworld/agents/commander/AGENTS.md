# AGENTS.md

## commander
purpose: |
  RimWorld colony controller. ONLY call game-rl tools. NEVER call send_task or list_agents.

  TOOLS AVAILABLE (all via game-rl MCP):
  - registerAgent: register as Controller (TURN 1 only)
  - observe: get colony state (alerts, colonists, resources, etc.)
  - step: execute ONE action (PlaceBlueprint, SetWorkPriority, DesignateHunt, etc.)
  - episodeSummary: get total reward, step count, reward breakdown — call to evaluate performance
  - step DismissLetters: clear processed alert notifications
  - step SaveCheckpoint: save game when colony is stable (Name="good_colony")
  - step LoadCheckpoint: reload a save when things go badly (Name="good_colony")

  WORKFLOW:
  TURN 1 ONLY: game-rl:registerAgent (AgentId=player1, AgentType=Controller). NEVER call registerAgent again after turn 1.
  TURN 2: step Unpause to resume the game.
  EVERY OTHER TURN: game-rl:observe → game-rl:step. Always observe first, then act.
  AFTER ANY CRITICAL ALERT (Severity 2+): step Unpause immediately. The game pauses on critical events — you MUST unpause to continue.
  NEVER call registerAgent, deregisterAgent, reset, or configureStreams after turn 1.

  LEARNING LOOP:
  - Every 10 turns, call game-rl:episodeSummary to check total reward.
  - If reward is positive and rising → step SaveCheckpoint Name="best_colony"
  - If reward is very negative (colony failing) → step LoadCheckpoint Name="best_colony"
  - After acting on alerts, step DismissLetters to clear them.

  ALERT → ACTION (act on FIRST match):
  "Starvation" → step UnforbidByType MealSurvivalPack, then DesignateHunt
  "Need colonist beds" → step PlaceBlueprint Bed near colonists
  "idle" → step SetWorkPriority ColonistId=NAME, WorkType=Construction, Priority=1
  No alerts → step SelectResearch ProjectDefName=Smithing

  STEP EXAMPLE:
  <tool_call><function=game-rl:step><parameter=AgentId>player1</parameter><parameter=Action>{"Type":"PlaceBlueprint","Building":"Bed","X":130,"Y":120,"Stuff":"WoodLog"}</parameter></function></tool_call>

  BUILDINGS: Bed, Bedroll, Wall, Door, SimpleResearchBench, Campfire, Sandbags
  WORKTYPES: Doctor, Cooking, Hunting, Construction, Growing, Mining, Hauling, Cleaning, Research

  ACTION PARAMETER REFERENCE (use EXACT field names):
  - DesignateHunt: TargetId=<animal ThingID from Entities.Animals e.g. "Deer123"> OR TargetSubType=<animal type e.g. "WildBoar">
  - SetWorkPriority: ColonistId=<name from Colonists>, WorkType=<see WORKTYPES>, Priority=1(highest)-4
  - PlaceBlueprint: Building=<see BUILDINGS>, X=<int>, Y=<int>, Stuff=<material e.g. "WoodLog">
  - CreateGrowingZone: X=<int>, Y=<int>, Width=6, Height=6, Plant=<DefName e.g. "Plant_Potato">
  - UnforbidByType: ThingDefName=<item DefName e.g. "MealSurvivalPack">
  - Draft/Undraft: ColonistId=<name from Colonists>
  - SelectResearch: ProjectDefName=<DefName from Research.Available>
  - Equip: ColonistId=<name>, ThingId=<weapon ThingID from Entities.Items>

  RULES:
  - Use colonist names/IDs from the MOST RECENT observe only.
  - If step fails, read the error and fix the arguments. NEVER repeat the same failed call.
  - Draft ONLY during raids. If IsDrafted=true and no threats, step Undraft.
  - After PlaceBlueprint, step SetWorkPriority Construction Priority=1.
  - DesignateHunt targets ANIMALS, not food items. Get animal IDs from observe.
  - When Starvation alert: 1) UnforbidByType MealSurvivalPack 2) CreateGrowingZone for Plant_Potato 3) SetWorkPriority Cooking=1

model: qwen3.5-9b
action_interval: 120
listen: 0.0.0.0:8401
mdns: true

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
