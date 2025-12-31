# RimWorld Survival Swarm

This example demonstrates a 5-agent HRM (Hierarchical Reasoning Model) swarm managing a RimWorld colony through the game-rl mod.

## The Story

Five AI agents coordinate to manage a RimWorld colony. A Commander agent holds the MCP tools and executes actions, while consulting specialized agents (Survival, Industry, Defense) via a Router that uses Thompson Sampling for optimal specialist selection.

## Why This Matters

1. **Multi-Agent Coordination**: HRM architecture with Commander, Router, and Specialists
2. **Colony Management**: Strategic decisions across survival, production, and defense
3. **A2A Protocol**: Agent-to-Agent communication via mDNS discovery
4. **Real-time Environment**: Decisions and actions in a live RimWorld colony

## Architecture

```
                    ┌─────────────────────┐
                    │     Commander       │ ← Has MCP tools
                    │ Port 8401           │   Executes colony actions
                    │ Model: ministral-3b │
                    └──────────┬──────────┘
                               │ A2A
                    ┌──────────▼──────────┐
                    │       Router        │ ← Thompson Sampling
                    │ Port 8402           │   Selects specialist
                    │ Model: ministral-3b │
                    └──────────┬──────────┘
           ┌───────────────────┼───────────────────┐
           │                   │                   │
    ┌──────▼──────┐     ┌──────▼──────┐     ┌──────▼──────┐
    │   Survival  │     │   Industry  │     │   Defense   │
    │ Port 8410   │     │ Port 8411   │     │ Port 8412   │
    │ ministral-3b│     │ ministral-3b│     │ ministral-3b│
    └─────────────┘     └─────────────┘     └─────────────┘
    Food, Health        Work Priorities     Combat, Raids
    Temperature         Production          Fortification
```

**RimWorld Colony** ← Commander executes actions via harmony-bridge → game-rl mod

## Quick Start

### Prerequisites

```bash
# Build Arkavo Edge
cd /Users/paul/Projects/arkavo/arkavo-edge
cargo build -p arkavo

# Build harmony-server (in game-rl repo)
cd /Users/paul/Projects/intelligence/game-rl
cargo build -p harmony-bridge

# Install Arkavo Game-RL mod
ln -s "$(pwd)/adapters/rimworld" ~/Library/Application\ Support/Steam/steamapps/common/RimWorld/RimWorldMac.app/Mods/ArkavoGameRL
```

### Run the Demo

```bash
# 1. Launch RimWorld with Arkavo Game-RL mod enabled
#    Start a new colony or load existing game
#    Mod creates socket at /tmp/gamerl-rimworld.sock

# 2. Start the agent swarm
./launch_rimworld.sh

# 3. Check swarm status
./launch_rimworld.sh status

# 4. Watch agents coordinate colony management
#    Commander receives objectives, consults specialists, executes actions

# 5. Stop agents
./launch_rimworld.sh stop
```

See [RUNBOOK.md](RUNBOOK.md) for detailed test procedures.

## Directory Structure

```
rimworld/
├── README.md                  # This file
├── RUNBOOK.md                 # Detailed test procedures
├── launch_rimworld.sh         # Start 5-agent swarm
├── stop_rimworld.sh           # Stop everything
├── agents/
│   ├── commander/
│   │   └── AGENTS.md          # MCP tools + coordination
│   ├── router/
│   │   └── AGENTS.md          # Specialist selection
│   └── specialists/
│       ├── survival/
│       │   └── AGENTS.md      # Food, health, mood
│       ├── industry/
│       │   └── AGENTS.md      # Work, production
│       └── defense/
│           └── AGENTS.md      # Combat, raids
└── logs/
    ├── commander.log
    ├── router.log
    ├── survival.log
    ├── industry.log
    └── defense.log
```

## Agent Roles

| Agent | Model | Port | Purpose |
|-------|-------|------|---------|
| Commander | ministral-3b | 8401 | Has MCP tools, executes colony actions, coordinates swarm |
| Router | ministral-3b | 8402 | Thompson Sampling specialist selection |
| Survival | ministral-3b | 8410 | Food, health, mood, temperature management |
| Industry | ministral-3b | 8411 | Work priorities, mining, construction, farming |
| Defense | ministral-3b | 8412 | Combat, drafting, threat response |

## MCP Tools (Commander Only)

| Tool | Description |
|------|-------------|
| `register_agent` | Register with RimWorld environment |
| `sim_step` | Execute action and advance simulation |
| `reset` | Reset episode |

### RimWorld Actions (via sim_step)

| Action | Description |
|--------|-------------|
| `SetWorkPriority` | Set colonist work priority (0-4) |
| `Draft` / `Undraft` | Direct pawn control for combat |
| `Move` / `MoveToEntity` | Move drafted pawns |
| `Attack` | Attack target with drafted pawn |
| `DesignateHunt` | Mark animal for hunting |
| `PlaceBlueprint` | Plan construction |
| `CreateGrowingZone` | Create farming zone |
| `CreateStockpile` | Create storage zone |
| `DesignateMine` | Designate mining area |
| `AddBill` | Add production bill to workbench |

## Coordination Flow

1. **Commander** receives colony management objective
2. **Commander** consults **Router** for specialist selection
3. **Router** uses Thompson Sampling to pick optimal specialist
4. **Specialist** (Survival/Industry/Defense) provides domain advice
5. **Commander** executes MCP tool calls based on advice
6. Repeat for each sub-task

## Transport

- **A2A**: Agent-to-Agent JSON-RPC over HTTP with mDNS discovery
- **MCP**: stdio transport via harmony-bridge to RimWorld socket

## Observation Data

The commander receives rich observation data including:
- Colonist states (mood, health, hunger, skills, current job)
- Resource stockpiles and wealth
- Weather, season, temperature
- Active threats and visitors
- Entity index (animals, buildings, items)
- Alerts and episode progress

## Reward Components

The environment provides multi-objective rewards:
- `colonist_death`: Major penalty for deaths
- `mood`: Average colonist happiness
- `hunger`/`exhaustion`/`health`: Basic needs
- `idle`: Penalty for unproductive colonists
- `wealth`/`food_security`: Economic indicators
- `threat_eliminated`/`threat_appeared`: Security events
- `research`/`construction`: Progress rewards
