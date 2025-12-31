# RimWorld Survival Swarm - Runbook

## Prerequisites Checklist

- [ ] Rust toolchain installed (rustc 1.85+)
- [ ] RimWorld installed via Steam
- [ ] Harmony mod enabled in RimWorld
- [ ] Arkavo Game-RL mod installed and enabled
- [ ] Arkavo Edge built (`cargo build -p arkavo`)
- [ ] harmony-bridge built (`cargo build -p harmony-bridge` in game-rl repo)
- [ ] Local LLM server running with Ministral-3B model

## Test Procedures

### T1: Basic Agent Startup

**Goal**: Verify all 5 agents start and become healthy.

```bash
# 1. Start RimWorld first (creates socket)
# Launch RimWorld, enable Arkavo Game-RL mod, start new colony

# 2. Verify socket exists
ls -la /tmp/gamerl-rimworld.sock

# 3. Start agent swarm
./launch_rimworld.sh

# 4. Verify all agents healthy
./launch_rimworld.sh status

# Expected: All 5 agents show "healthy"
# - commander:8401
# - router:8402
# - survival:8410
# - industry:8411
# - defense:8412
```

### T2: MCP Connection

**Goal**: Verify commander can connect to RimWorld via harmony-bridge.

```bash
# 1. Check commander logs for MCP connection
tail -f logs/commander.log

# Expected:
# - "Spawning MCP server: rimworld"
# - "MCP server rimworld ready"
# - No connection errors
```

### T3: Agent Registration

**Goal**: Verify commander registers with RimWorld.

```bash
# 1. Send registration request to commander
curl -X POST http://localhost:8401/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Register with RimWorld as colony_manager"}'

# 2. Check response includes successful registration
# Expected: Agent registered, observation received
```

### T4: Colony Observation

**Goal**: Verify commander can observe colony state.

```bash
# 1. Request colony status
curl -X POST http://localhost:8401/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Observe the colony and tell me the current state"}'

# Expected response includes:
# - Number of colonists
# - Resource counts
# - Weather/season
# - Any threats or alerts
```

### T5: Specialist Consultation

**Goal**: Verify A2A communication between commander and specialists.

```bash
# 1. Ask about food situation (should consult Survival specialist)
curl -X POST http://localhost:8401/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Are we at risk of starvation? What should we prioritize?"}'

# 2. Check router logs for Thompson Sampling selection
tail logs/router.log

# 3. Check survival specialist logs for consultation
tail logs/survival.log

# Expected: Router selects Survival, specialist provides advice
```

### T6: Work Priority Assignment

**Goal**: Verify commander can assign work priorities.

```bash
# 1. Request work assignment
curl -X POST http://localhost:8401/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Set the first colonist to priority 1 for Hunting"}'

# 2. Verify in RimWorld UI that work priority changed

# Expected: Work priority visibly updates in game
```

### T7: Threat Response

**Goal**: Verify defense specialist advises on threats.

```bash
# 1. Wait for or trigger a raid in RimWorld

# 2. Ask about threat
curl -X POST http://localhost:8401/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "There are raiders attacking! What should we do?"}'

# Expected: Router selects Defense, specialist provides combat advice
# Commander drafts colonists and positions for defense
```

### T8: Construction Planning

**Goal**: Verify industry specialist advises on building.

```bash
# 1. Ask about construction
curl -X POST http://localhost:8401/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "We need shelter. What buildings should we construct first?"}'

# Expected: Router selects Industry, specialist provides building advice
# Commander places blueprints
```

## Troubleshooting

### Socket Not Found

```bash
# Error: /tmp/gamerl-rimworld.sock not found

# Solution: Ensure RimWorld is running with GameRL mod enabled
# Check RimWorld logs for mod initialization
```

### Agent Won't Start

```bash
# Check if port is in use
lsof -i :8401

# Kill existing processes
./stop_rimworld.sh
pkill -f arkavo

# Restart
./launch_rimworld.sh
```

### MCP Connection Failed

```bash
# Check harmony-bridge can connect
/Users/paul/Projects/intelligence/game-rl/target/debug/harmony-bridge /tmp/gamerl-rimworld.sock

# If fails, check RimWorld is responsive
# Try pausing game, mod may need active game tick
```

### No Response from Specialists

```bash
# Check A2A connectivity
curl http://localhost:8402/health  # router
curl http://localhost:8410/health  # survival

# Check mDNS discovery
# Logs should show peer discovery
tail logs/router.log | grep -i peer
```

## Log Locations

```
logs/
├── commander.log   # MCP calls, action execution
├── router.log      # Thompson Sampling decisions
├── survival.log    # Food/health advice
├── industry.log    # Work/production advice
└── defense.log     # Combat/threat advice
```

## Stop Procedure

```bash
# Graceful shutdown
./launch_rimworld.sh stop

# Force cleanup
./stop_rimworld.sh

# Verify stopped
./launch_rimworld.sh status
```
