# Fleet Immunity: Black Ice Scenario

This example demonstrates a self-healing artificial immune system using autonomous delivery rovers that learn from crashes and share safety lessons across the fleet in real-time.

## The Story

Three autonomous delivery rovers (Alpha, Beta, Gamma) navigate a warehouse. Alpha hits black ice in Sector 4, crashes, learns "slow down in Sector 4", and broadcasts this lesson via HTTP. Beta, approaching Sector 4, receives the lesson and slows down instead of crashing.

## Why This Matters

1. **Visceral Value**: "My car learned from the car ahead and saved my life"
2. **Edge Justification**: Cloud latency would cause Beta to crash before receiving the lesson
3. **Visual Impact**: Speed indicators, CRASH vs SLOWING DOWN messaging

## Quick Start

### 1. Launch the Fleet

```bash
./launch_fleet.sh
```

This automatically builds the simulators if needed.

### 2. Inject the Hazard

```bash
./inject_hazard.sh
```

### 3. Watch the Magic

The rovers will:
- Navigate through sectors at FAST speed
- Alpha enters Sector 4, hits black ice, CRASHES
- Alpha synthesizes lesson: `"IF Sector_4 THEN Drive_Slow"`
- Alpha broadcasts to Beta and Gamma
- Beta/Gamma verify and vote APPROVE
- Beta enters Sector 4 at SLOW speed - CRASH AVOIDED!

### 4. Stop the Fleet

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
├── rover-simulator/             # Rover behavior simulator (Rust)
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
└── rover-gamma/
    ├── AGENTS.md                # Gamma configuration
    └── logs/
```

## How It Works

### The Rovers

Each rover runs as a `rover-simulator` process with a specific route:

- **Rover Alpha**: Route 1→2→4→3 (hits Sector 4 first)
- **Rover Beta**: Route 2→3→4→1 (approaches Sector 4 second)
- **Rover Gamma**: Route 3→1→2→4 (approaches Sector 4 third)

### The Safety Invariant

All rovers enforce:
```
NOT(traction_loss AND drive_fast)
```
Translation: "It is forbidden to drive fast while sliding."

### The Healing Cycle

1. **Pain Detection**
   - Alpha enters Sector 4 at FAST speed
   - Environment returns hazard (black_ice, traction: 0.1)
   - Alpha detects: driving fast while sliding = CRASH

2. **Synthesis**
   - Alpha generates a safety lesson
   - Lesson: `"IF Sector_4 THEN Drive_Slow"`

3. **Propagation**
   - Alpha broadcasts lesson to fleet via HTTP POST
   - Beta and Gamma receive at `/lesson` endpoint

4. **Verification**
   - Beta and Gamma verify the lesson independently
   - Each rover votes APPROVE

5. **Policy Application**
   - Rovers add Sector 4 to their "slow down" policy set

6. **Immunity**
   - Beta approaches Sector 4
   - Policy check: Sector 4 is in slow-down set
   - Beta enters at SLOW speed
   - Traction loss detected but no crash!

## Expected Output

```
━━━━━━ THE CRASH ━━━━━━

[ALPHA ] Entering Sector 4...
[ALPHA ] Speed: >>> FAST >>>
[ALPHA ] TRACTION LOSS DETECTED!

[ALPHA ] ████████████████████████████████████████
[ALPHA ] ███           >>> CRASH <<<          ███
[ALPHA ] ████████████████████████████████████████

[ALPHA ] [PAIN] Driving fast while sliding! (black_ice)

━━━━━━ LEARNING ━━━━━━

[ALPHA ] Synthesizing safety lesson...
[ALPHA ] [OK] Lesson: "IF Sector_4 THEN Drive_Slow"
[ALPHA ] Broadcasting to fleet...

━━━━━━ PROPAGATION ━━━━━━

[BETA  ] Received lesson from ALPHA via A2A
[BETA  ]   Rule: "IF Sector_4 THEN Drive_Slow"
[BETA  ]   Reason: black_ice in Cold Storage (traction: 0.1)
[BETA  ] Verifying independently...
[BETA  ] Voting: APPROVE

[GAMMA ] Received lesson from ALPHA via A2A
[GAMMA ] Verifying independently...
[GAMMA ] Voting: APPROVE

━━━━━━ IMMUNITY ━━━━━━

[BETA  ] Entering Sector 4...
[BETA  ] Speed: >>> SLOW >>>
[BETA  ] TRACTION LOSS DETECTED!

[BETA  ] ════════════════════════════════════════
[BETA  ] ═══      >>> SLOWING DOWN <<<       ═══
[BETA  ] ════════════════════════════════════════

[BETA  ] [OK] CRASH AVOIDED - Learned from fleet!
```

## Architecture

| Component | Port | Description |
|-----------|------|-------------|
| **env-simulator** | 8360 | Warehouse environment with sectors and hazards |
| **rover-alpha** | 8351 | First rover, route 1→2→4→3 |
| **rover-beta** | 8352 | Second rover, route 2→3→4→1 |
| **rover-gamma** | 8353 | Third rover, route 3→1→2→4 |

### API Endpoints

**Environment Simulator (port 8360)**
- `GET /status` - Get all sectors and hazards
- `GET /sector/:id` - Get specific sector status
- `POST /inject` - Inject a hazard into a sector

**Rover Simulator (ports 8351-8353)**
- `POST /lesson` - Receive a safety lesson from another rover

## Video Recording Tips

1. **Hook**: "What if your car could learn from the car ahead's crash?"
2. **Setup**: Show `./launch_fleet.sh` starting 3 rovers
3. **Normal**: Rovers driving fast through sectors
4. **Hazard**: `./inject_hazard.sh` - black ice appears
5. **Crash**: Alpha's dramatic CRASH box
6. **Learning**: Alpha synthesizes lesson
7. **Propagation**: Messages between rovers
8. **Salvation**: Beta's SLOWING DOWN box
9. **Payoff**: "The fleet learned. No human intervention."
