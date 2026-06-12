# Zomboid Survival Kit

A SwarmKit that plays Project Zomboid as a hub-spoke survivor swarm. One `survivor` role is the embodied agent that holds the game-rl MCP grant and drives every in-game action; four supporting roles — `threat`, `scavenger`, `medic`, and `critic` — operate purely through A2A messaging, advising or evaluating the survivor without touching the game directly.

## Roles

| Role | What it does |
|---|---|
| `survivor` | Hub agent. Drives Project Zomboid each cycle via game-rl (`observe` / `step`). Weighs advisor input and executes exactly one tool call per cycle. |
| `threat` | Advisor. Reads the survivor's relayed zombie observations and recommends attack, shove-and-flee, or repositioning. |
| `scavenger` | Advisor. Reads relayed nearby items and the rendered map; recommends the highest-value loot and a safe route to it. |
| `medic` | Advisor. Reads relayed survivor stats; recommends when to eat, drink, rest, or break contact to treat injuries. |
| `critic` | Evaluator. Scores the completed episode against the survival rubric and flags cycles where critical alerts were ignored. |

Only `survivor` connects to the game. The three advisors (`threat`, `scavenger`, `medic`) exchange A2A recommendations with the survivor, the `critic` evaluates the episode, and none of these four call game-rl tools directly.

## The kit is the source of truth

The kit is a single content-addressed YAML file. `kit.id` is a BLAKE3 hash of its canonical form — any edit changes the identity. The game-rl MCP server is declared inside the kit under `mcp_servers`:

```yaml
mcp_servers:
  - name: "game-rl"
    command: "${GAME_RL_SERVER}"
    args: []
    transport: "stdio"
```

Because the server declaration is part of the kit, it is part of the kit's identity and provenance. Validate the kit with:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- examples/zomboid-survival-kit/zomboid-survival-kit.swarmkit.yaml
```

## How it runs

`arkavo swarmkit play <kit>` maps each role to the existing agent runtime (`start_agent_server`) and ticks the live MCP loop. The `survivor` connects to game-rl and calls `registerAgent` → `observe` / `step` each cycle; the advisors receive the relayed observation and reply via A2A before the next cycle begins.

This is different from the panel-only path (`ARKAVO_SWARMKIT_PATH` + `arkavo ui`), which only visualizes a kit's structure in the UI and does **not** drive a live game loop.

## Prerequisites

1. **Project Zomboid** installed with the GameRL Lua mod symlinked into `~/Zomboid/mods/GameRL`, and a save loaded so `~/Zomboid/Lua/gamerl_response.json` exists.
2. **game-rl-server** built (`cargo build --release` in the game-rl repo at `~/Projects/intelligence/game-rl`) or available on `PATH`.
3. **arkavo** built: `cargo build -p arkavo` in this repo.

## Run

```bash
./examples/zomboid-survival-kit/run-kit.sh
```

Set `GAME_RL_SERVER=/path/to/game-rl-server` to override the server binary path.

---

`examples/zomboid/` is the simpler single-agent (non-kit) version of the same game.
