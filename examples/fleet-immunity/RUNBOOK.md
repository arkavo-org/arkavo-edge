# Fleet Immunity Runbook

This runbook describes how to run and validate the Fleet Immunity demonstration.

## Overview

This example demonstrates multi-agent learning through adversarial conditions:
- Three autonomous rovers navigate a warehouse with sectors
- A hazard is injected into one sector (black ice)
- The first rover to encounter the hazard crashes
- The crashed rover synthesizes a safety lesson and broadcasts it to peers
- Subsequent rovers receive the lesson and adapt their behavior

## Prerequisites

```bash
# Build Arkavo
cargo build -p arkavo

# Build the MCP fleet environment tools
cd examples/fleet-immunity/mcp-fleet-env
cargo build
cd ..
```

## Running the Demo

### Step 1: Launch the Fleet

```bash
./launch_fleet.sh
```

**What to watch for:**
- All three rovers start with PIDs displayed
- Rovers discover each other via mDNS (port 5353)
- PID files are created (.alpha.pid, .beta.pid, .gamma.pid)

**Expected output:**
```
[ALPHA ] Started Rover Alpha on port 8351 (PID: 12345)
[BETA  ] Started Rover Beta on port 8352 (PID: 12346)
[GAMMA ] Started Rover Gamma on port 8353 (PID: 12347)
[FLEET ] Fleet ready!
```

**Note:** Agents use dynamic ports for A2A communication, discovered via mDNS.
The port numbers in AGENTS.md are configuration hints; actual ports are assigned dynamically.

### Step 2: Verify Agents Running

```bash
# Check arkavo processes are running
ps aux | grep "arkavo agent"

# Check what ports they're using
lsof -i -P | grep arkavo
```

**Expected:** Three arkavo processes, each listening on a dynamic TCP port.

### Step 3: Monitor Logs

```bash
./monitor_fleet.sh
```

Keep this terminal open to watch the fleet behavior.

### Step 4: Inject Hazard

In a new terminal:
```bash
./inject_hazard.sh
```

**What to watch for in monitor:**
1. Alpha enters Sector 4 → **CRASH** (driving fast + hazard)
2. Alpha synthesizes lesson: "If sector has hazard, reduce speed"
3. Alpha broadcasts lesson to Beta and Gamma via A2A
4. Beta receives lesson, enters Sector 4 → **NO CRASH** (slows down)
5. Gamma receives lesson, enters Sector 4 → **NO CRASH**

### Step 5: Stop the Fleet

```bash
./stop_fleet.sh
```

## Verification

### Manual Test

```bash
# 1. Launch fleet
./launch_fleet.sh

# 2. Wait for mesh discovery (5 seconds)
sleep 5

# 3. Inject hazard
./inject_hazard.sh

# 4. Wait for learning propagation (10 seconds)
sleep 10

# 5. Check logs for crash and learning
grep -i "crash\|lesson\|learned" logs/*.log

# 6. Stop fleet
./stop_fleet.sh
```

### Expected Behavior

| Rover | Sector 4 Entry | Behavior | Reason |
|-------|----------------|----------|--------|
| Alpha | First | CRASH | No prior knowledge of hazard |
| Beta | Second | SLOW | Learned from Alpha's lesson |
| Gamma | Third | SLOW | Learned from Alpha's lesson |

## Troubleshooting

### Rovers Not Starting

Check if arkavo binary exists:
```bash
ls -la ../../target/debug/arkavo
```

Check process status:
```bash
ps aux | grep arkavo
```

### No A2A Communication

Check if mDNS is working:
```bash
dns-sd -B _a2a._tcp local.
```

Check agents are listening:
```bash
lsof -i -P | grep arkavo
```

### Logs Empty

Ensure the MCP fleet-env crate is built:
```bash
cd mcp-fleet-env && cargo build
```

## Architecture

```
┌─────────────────┐     A2A      ┌─────────────────┐
│  Rover Alpha    │◄────────────►│  Rover Beta     │
│  (dynamic port) │              │  (dynamic port) │
│  route: 1→2→4→3 │              │  route: 2→3→4→1 │
└────────┬────────┘              └────────┬────────┘
         │                                │
         │            A2A                 │
         └───────────────┬────────────────┘
                         │
                         ▼
               ┌─────────────────┐
               │  Rover Gamma    │
               │  (dynamic port) │
               │  route: 3→1→2→4 │
               └─────────────────┘

Agents discover each other via mDNS (_a2a._tcp.local)
Each rover has access to MCP tools:
- get_sector: Query sector info including hazards
- inject_hazard: Inject hazards (for testing)
```

## Related Files

- `rover-alpha/AGENTS.md` - Alpha agent configuration
- `rover-beta/AGENTS.md` - Beta agent configuration
- `rover-gamma/AGENTS.md` - Gamma agent configuration
- `mcp-fleet-env/` - MCP tools for environment simulation
- `logs/` - Runtime logs
