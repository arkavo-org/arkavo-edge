# GridColony — headless Game-RL playtest example

GridColony is the Game-RL **reference environment** (`game-rl-reference`, spec
draft-02 conformance Level 3) from the [game-rl](https://github.com/arkavo-ai/game-rl)
repository. It is deterministic, headless, and instant to reset — the ideal
target for Edge agent playtesting without launching a real game.

## Why this example exists

The `fertile-corner` scenario is a **behavioral probe for spatial decision
quality**: all fertile soil is in the NE corner, far from the map center. An
agent that anchors placements to `MapCenter` (the historical center-bias
failure) scores **zero** on the `farm_fertility_quality` reward component; an
agent that reads `Landmarks` and anchors to `FertileCluster_0` scores ~1.3.
Use it as a regression test for spatial policy quality.

## Scenarios

| Scenario | Probe |
|----------|-------|
| `default` | Balanced colony start; fertile soil near (but not at) center |
| `fertile-corner` | **Anti-center-bias probe** — fertile soil only in the NE |
| `threat-south` | Hostiles approach from the south; tests alert→DefendColony |
| `scattered-resources` | Resources in all compass regions; tests landmark navigation |

Append `+dr` to any scenario name for domain-randomized fertility.

## Run the scripted policy probe (no LLM required)

```bash
# Build the reference env first (in the game-rl repo):
#   cargo build --release -p game-rl-reference
python3 playtest_probe.py            # runs center-biased vs landmark-aware policies
python3 playtest_probe.py --steps 40 # longer episodes
```

This drives both policies through every scenario over real MCP stdio and
reports `farm_fertility_quality` and `TotalReward` side by side.

## Run the LLM agent swarm

```bash
export GAME_RL_REFERENCE=~/Projects/intelligence/game-rl/target/release/game-rl-reference
export GRIDCOLONY_SCENARIO=fertile-corner
./launch_gridcolony.sh
```

Requires a local model (see `agents/commander/AGENTS.md`) or router-configured
provider. The commander's prompt teaches the draft-02 spatial workflow:
discover anchors via `observe(Include=["landmarks","terrain"])`, preview with
`resolveSpatial`, and never anchor farms to `MapCenter`.
