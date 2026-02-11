# Minecraft Survival Swarm

<!-- ARKAVO-CAPABILITY: mcp-tools -->
> **Specs**: [10 scenarios](../../specs/arkavo-edge/mcp-tools.spec.yaml)
> **Browse**: `cargo xtask capabilities mcp-tools`
<!-- /ARKAVO-CAPABILITY -->

This example demonstrates a 5-agent HRM (Hierarchical Reasoning Model) swarm controlling a single Minecraft bot in vanilla Survival mode.

## The Story

Five AI agents coordinate to control a Minecraft bot named "Edge" in a survival scenario. A Commander agent holds the MCP tools and executes actions, while consulting specialized agents (Scout, Builder, Runner) via a Router that uses Thompson Sampling for optimal specialist selection.

## Why This Matters

1. **Multi-Agent Coordination**: HRM architecture with Commander, Router, and Specialists
2. **Shared Resource Control**: Multiple agents coordinate around a single bot
3. **A2A Protocol**: Agent-to-Agent communication via mDNS discovery
4. **Real-time Environment**: Decisions and actions in a live Minecraft world

## Architecture

```
                    ┌─────────────────────┐
                    │     Commander       │ ← Has MCP tools
                    │ Port 8401           │   Executes bot actions
                    │ Model: ministral-8b │
                    └──────────┬──────────┘
                               │ A2A
                    ┌──────────▼──────────┐
                    │       Router        │ ← Thompson Sampling
                    │ Port 8402           │   Selects specialist
                    │ Model: qwen3-0.6b   │
                    └──────────┬──────────┘
           ┌───────────────────┼───────────────────┐
           │                   │                   │
    ┌──────▼──────┐     ┌──────▼──────┐     ┌──────▼──────┐
    │    Scout    │     │   Builder   │     │   Runner    │
    │ Port 8410   │     │ Port 8411   │     │ Port 8412   │
    │ qwen3-0.6b  │     │ ministral-3b│     │ qwen3-0.6b  │
    └─────────────┘     └─────────────┘     └─────────────┘
    Navigation          Resources           Objectives
    Threat detection    Construction        Escape routes
```

**Single Bot "Edge"** ← Commander executes all bot actions based on specialist advice

## Quick Start

### Prerequisites

```bash
# Build Arkavo
cargo build -p arkavo

# Install Docker (for Minecraft server)
# macOS: brew install --cask docker
# Linux: https://docs.docker.com/engine/install/
```

### Run the Demo

```bash
# 1. Start swarm (Minecraft server + 5 agents)
./launch_minecraft.sh

# 2. Check swarm status
./launch_minecraft.sh status

# 3. In Minecraft client, connect to localhost:25565
#    Watch Edge respond to coordinated agent commands

# 4. Stop everything
./launch_minecraft.sh stop
# or
./stop_minecraft.sh
```

See [RUNBOOK.md](RUNBOOK.md) for detailed test procedures.

## Directory Structure

```
minecraft/
├── README.md                  # This file
├── RUNBOOK.md                 # Detailed test procedures
├── compose.yaml               # Docker Compose for Minecraft server
├── launch_minecraft.sh        # Start server and 5-agent swarm
├── stop_minecraft.sh          # Stop everything
├── agents/
│   ├── commander/
│   │   └── AGENTS.md          # MCP tools + coordination
│   ├── router/
│   │   └── AGENTS.md          # Specialist selection
│   └── specialists/
│       ├── scout/
│       │   └── AGENTS.md      # Navigation advice
│       ├── builder/
│       │   └── AGENTS.md      # Construction advice
│       └── runner/
│           └── AGENTS.md      # Objective advice
└── logs/
    ├── commander.log
    ├── router.log
    ├── scout.log
    ├── builder.log
    └── runner.log
```

## Agent Roles

| Agent | Model | Port | Purpose |
|-------|-------|------|---------|
| Commander | ministral-8b | 8401 | Has MCP tools, executes bot actions, coordinates swarm |
| Router | qwen3-0.6b | 8402 | Thompson Sampling specialist selection |
| Scout | qwen3-0.6b | 8410 | Navigation, exploration, threat detection |
| Builder | ministral-3b | 8411 | Resource gathering, construction, crafting |
| Runner | qwen3-0.6b | 8412 | Objective retrieval, escape planning |

## MCP Tools (Commander Only)

| Tool | Description |
|------|-------------|
| `get-position` | Get bot's current coordinates |
| `move-to-position` | Navigate to x,y,z |
| `look-at` | Look at a position |
| `dig-block` | Mine a block |
| `place-block` | Place a block |
| `list-inventory` | List inventory contents |
| `find-block` | Find nearest block type |
| `find-entity` | Find nearest entity |
| `send-chat` | Send chat message |

## Coordination Flow

1. **Commander** receives survival objective
2. **Commander** consults **Router** for specialist selection
3. **Router** uses Thompson Sampling to pick optimal specialist
4. **Specialist** (Scout/Builder/Runner) provides domain advice
5. **Commander** executes MCP tool calls based on advice
6. Repeat for each sub-task

## Transport

- **A2A**: Agent-to-Agent JSON-RPC over HTTP with mDNS discovery
- **MCP**: stdio transport via Docker exec to minecraft-mcp-server

## Joining the Game

1. Open Minecraft Java Edition (version 1.21.x)
2. Multiplayer > Direct Connect > `localhost:25565`
3. Join the server (offline mode, no Mojang auth needed)
4. Find Edge and watch the swarm coordinate actions
