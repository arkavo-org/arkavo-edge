# Fleet Immunity: Black Ice Scenario

This example demonstrates Arkavo Edge's self-healing artificial immune system using autonomous delivery rovers that learn from crashes and share safety lessons across the fleet in real-time.

## The Story

Three autonomous delivery rovers (Alpha, Beta, Gamma) navigate a warehouse. Alpha hits black ice in Sector 4, crashes, learns "slow down in Sector 4", and broadcasts this lesson via A2A. Beta, approaching Sector 4, receives the patch just in time and slows down instead of crashing.

## Why This Matters

1. **Visceral Value**: "My car learned from the car ahead and saved my life"
2. **Edge Justification**: Cloud latency would cause Beta to crash before receiving the lesson - it *must* be local mesh gossip
3. **Visual Impact**: Speed indicators, CRASH vs SLOWING DOWN messaging

## Quick Start

### 1. Build Arkavo

```bash
cd ../..
cargo build
```

### 2. Launch the Fleet

```bash
./launch_fleet.sh
```

### 3. Monitor the Fleet

```bash
./monitor_fleet.sh
```

### 4. Inject the Hazard

```bash
./inject_hazard.sh
```

### 5. Watch the Magic

- Alpha enters Sector 4 at high speed
- Alpha crashes (traction loss while driving fast)
- Alpha synthesizes patch: "IF Sector_4 THEN Drive_Slow"
- Alpha broadcasts to fleet via A2A
- Beta/Gamma verify and vote APPROVE
- Beta enters Sector 4 and SLOWS DOWN - no crash!

### 6. Stop the Fleet

```bash
./stop_fleet.sh
```

## Directory Structure

```
fleet-immunity/
├── README.md                    # This file
├── launch_fleet.sh              # Start all 3 rovers
├── stop_fleet.sh                # Stop all rovers
├── inject_hazard.sh             # Inject black ice hazard
├── monitor_fleet.sh             # Watch fleet logs
├── env-simulator/               # Warehouse environment simulator (Rust)
│   ├── Cargo.toml
│   └── src/main.rs
├── environment/
│   ├── warehouse.yaml           # Sector definitions
│   └── routes.yaml              # Delivery routes
├── rover-alpha/
│   ├── AGENTS.md                # Alpha configuration
│   └── logs/
├── rover-beta/
│   ├── AGENTS.md                # Beta configuration
│   └── logs/
├── rover-gamma/
│   ├── AGENTS.md                # Gamma configuration
│   └── logs/
└── scenarios/
    ├── black-ice.yaml           # Black ice injection
    └── normal-operation.yaml    # Baseline test
```

## How It Works

### The Rovers

Each rover is an Arkavo Edge agent configured via `AGENTS.md`:

- **Rover Alpha**: Route 1→2→4→3 (hits Sector 4 first)
- **Rover Beta**: Route 2→3→4→1 (approaches Sector 4 second)
- **Rover Gamma**: Route 3→1→2→4 (approaches Sector 4 third)

### The Safety Invariant

All rovers share the same invariant (SBE):
```
NOT(traction_loss AND drive_fast)
```
Translation: "It is forbidden to drive fast while sliding."

### The Healing Cycle

1. **Pain Detection** (Titan Monitor)
   - Alpha enters Sector 4 at high speed
   - Environment injects traction loss
   - Titan detects invariant violation: driving fast while sliding

2. **Synthesis** (Ministral-3B)
   - Alpha's local LLM generates a patch
   - Patch: "IF Sector_4 THEN Drive_Slow"
   - Verified against 500 test inputs

3. **Propagation** (A2A + Gossip)
   - Alpha broadcasts patch to fleet
   - Beta and Gamma receive via A2A protocol

4. **Zero-Trust Verification**
   - Beta and Gamma verify patch independently
   - Each agent runs its own SAT verification
   - They don't trust Alpha's LLM

5. **Quorum Consensus**
   - 2/3 majority required for adoption
   - All 3 rovers vote APPROVE
   - Patch is applied fleet-wide

6. **Immunity**
   - Beta approaches Sector 4
   - Policy check triggers: Sector_4 detected
   - Beta slows down BEFORE entering
   - No crash!

## Expected Output

```
━━━━━━ PHASE 1: FLEET INITIALIZATION ━━━━━━

[ALPHA ] Rover initialized - Delivery Route A
[ALPHA ] Registering on mesh: rover-alpha._fleet._tcp.local.
[BETA  ] Rover initialized - Delivery Route B
[GAMMA ] Rover initialized - Delivery Route C
[FLEET ] All rovers connected via A2A (3/3)

━━━━━━ PHASE 2: NORMAL OPERATION ━━━━━━

[ALPHA ] Sector 1 >>> FAST >>> [OK]
[BETA  ] Sector 2 >>> FAST >>> [OK]
[GAMMA ] Sector 3 >>> FAST >>> [OK]

━━━━━━ PHASE 3: THE CRASH ━━━━━━

[ALPHA ] Entering Sector 4...
[ALPHA ] Speed: >>> FAST >>>
[ALPHA ] TRACTION LOSS DETECTED!

[ALPHA ] ████████████████████████████████████████
[ALPHA ] ███           >>> CRASH <<<          ███
[ALPHA ] ████████████████████████████████████████

[ALPHA ] [PAIN] Titan: Driving fast while sliding!

━━━━━━ PHASE 4: LEARNING ━━━━━━

[ALPHA ] Synthesizing safety lesson...
[ALPHA ] [OK] Lesson: "IF Sector_4 THEN Drive_Slow"
[ALPHA ] Broadcasting to fleet via A2A...

━━━━━━ PHASE 5: FLEET PROPAGATION ━━━━━━

[BETA  ] Received lesson from ALPHA via A2A
[BETA  ] Verifying independently...
[BETA  ] Voting: APPROVE

[GAMMA ] Received lesson from ALPHA via A2A
[GAMMA ] Verifying independently...
[GAMMA ] Voting: APPROVE

[FLEET ] Quorum reached: 3/3 approved

━━━━━━ PHASE 6: IMMUNITY ━━━━━━

[BETA  ] Approaching Sector 4...
[BETA  ] Policy check: Sector_4 detected

[BETA  ] ════════════════════════════════════════
[BETA  ] ═══      >>> SLOWING DOWN <<<       ═══
[BETA  ] ════════════════════════════════════════

[BETA  ] [OK] CRASH AVOIDED - Learned from Alpha!

━━━━━━ FLEET IMMUNITY ACHIEVED ━━━━━━
```

## Key Talking Points

| Component | Description |
|-----------|-------------|
| **Titan Monitor** | Nervous system with 34ns overhead. Detects invariant violations. |
| **SBE** | Symbolic Boundary Evolution. Hierarchical policy layers. |
| **Ministral-3B** | Local edge LLM for patch synthesis. No cloud latency. |
| **A2A Protocol** | Agent-to-agent mesh communication. |
| **Gossip Protocol** | Quorum consensus (2/3 majority) for patch adoption. |

## Video Recording Tips

1. **Hook**: "What if your car could learn from the car ahead's crash?"
2. **Setup**: Show `./launch_fleet.sh` starting 3 rovers
3. **Normal**: Rovers driving fast through sectors
4. **Hazard**: `./inject_hazard.sh` - black ice appears
5. **Crash**: Alpha's dramatic CRASH box
6. **Learning**: Alpha synthesizes lesson
7. **Propagation**: A2A messages between rovers
8. **Salvation**: Beta's SLOWING DOWN box
9. **Payoff**: "The fleet learned. No human intervention."

## License

This example is part of the Arkavo project and follows the same license terms.
