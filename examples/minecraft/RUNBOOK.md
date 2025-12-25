# Minecraft Survival Swarm Runbook

This runbook describes how to run and validate the 5-agent Minecraft swarm.

## Overview

This example demonstrates:
- HRM (Hierarchical Reasoning Model) multi-agent architecture
- A2A protocol for agent-to-agent coordination
- Commander pattern with specialist delegation
- MCP stdio transport for Minecraft bot control

## Prerequisites

```bash
# Build Arkavo
cargo build -p arkavo

# Verify Docker is running
docker info

# Verify binary exists
ls -la ../../target/debug/arkavo
```

## Running the Demo

### Step 1: Start the Swarm

```bash
./launch_minecraft.sh
```

**What happens:**
1. Minecraft server starts in Docker
2. Waits for server health check
3. Starts 5 agents in dependency order:
   - Specialists: scout, builder, runner (parallel)
   - Router (needs specialists)
   - Commander (needs router)
4. Waits 5s for mDNS discovery
5. Verifies all agents are healthy

**Expected output:**
```
========================================
 MINECRAFT SURVIVAL SWARM
 5-Agent HRM Demo
========================================

[MINECRAFT] Starting Minecraft server...
[INFO] Waiting for server to be healthy...
[SUCCESS] Minecraft server is healthy!
[SUCCESS] MCP container ready!

[INFO] Starting HRM swarm agents...

[SWARM] Starting scout on port 8410...
[SUCCESS] scout started (PID: 12345)
[SWARM] Starting builder on port 8411...
[SUCCESS] builder started (PID: 12346)
[SWARM] Starting runner on port 8412...
[SUCCESS] runner started (PID: 12347)
[SWARM] Starting router on port 8402...
[SUCCESS] router started (PID: 12348)
[SWARM] Starting commander on port 8401...
[SUCCESS] commander started (PID: 12349)

[INFO] Waiting for mDNS discovery (5s)...

[INFO] Verifying swarm connectivity...
[SUCCESS] commander is healthy
[SUCCESS] router is healthy
[SUCCESS] scout is healthy
[SUCCESS] builder is healthy
[SUCCESS] runner is healthy

[SUCCESS] Minecraft Survival Swarm is ready!

Agent Endpoints:
  Commander: http://localhost:8401  (has MCP tools)
  Router:    http://localhost:8402
  Scout:     http://localhost:8410
  Builder:   http://localhost:8411
  Runner:    http://localhost:8412

Minecraft: localhost:25565 (connect with client)
```

### Step 2: Check Swarm Status

```bash
./launch_minecraft.sh status
```

**Expected:** All 5 agents report healthy.

### Step 3: View Agent Logs

```bash
# Commander (has MCP tools)
tail -f logs/commander.log

# Router decisions
tail -f logs/router.log

# Specialist advice
tail -f logs/scout.log
tail -f logs/builder.log
tail -f logs/runner.log
```

### Step 4: Connect Minecraft Client

1. Open Minecraft Java Edition 1.21.x
2. Multiplayer > Direct Connect > `localhost:25565`
3. Join and find the bot "Edge"

### Step 5: Stop the Swarm

```bash
./launch_minecraft.sh stop
# or
./stop_minecraft.sh
```

## Verification

### Health Check All Agents

```bash
for port in 8401 8402 8410 8411 8412; do
  echo -n "Port $port: "
  curl -s http://localhost:$port/health && echo "OK" || echo "FAIL"
done
```

### Verify MCP Tools on Commander

Check `logs/commander.log` for:
```
[MCP] Discovered 17 tools: ["get-position", "move-to-position", ...]
```

### Verify A2A Peers

Check `logs/router.log` for peer discovery:
```
[A2A] Discovered peers: 8401, 8410, 8411, 8412
```

## Troubleshooting

### Agent Fails to Start

```bash
# Check port in use
lsof -i :8401

# Check log for errors
cat logs/commander.log
```

### MCP Tools Not Discovered

```bash
# Verify MCP container is running
docker compose ps mcp

# Verify Docker exec works
docker exec arkavo-minecraft-mcp echo "test"
```

### Minecraft Server Not Ready

```bash
# Check Docker logs
docker compose logs minecraft

# Wait for "Done" message
docker compose logs -f minecraft | grep -m1 "Done"
```

### Agents Not Discovering Peers

```bash
# Increase mDNS wait time (edit launch_minecraft.sh)
# Or add explicit peers in AGENTS.md
```

## Architecture Notes

### Why HRM?

The Hierarchical Reasoning Model separates:
- **Strategic reasoning** (Commander) - what to do
- **Specialist selection** (Router) - who to ask
- **Domain knowledge** (Specialists) - how to do it

### Why Single Bot?

Multiple agents coordinate to control ONE bot because:
- Minecraft limits concurrent actions per entity
- Demonstrates collaborative AI without resource conflicts
- Commander serializes actions based on specialist consensus

### Agent Communication

- **A2A**: HTTP JSON-RPC on local ports
- **mDNS**: Zero-config peer discovery
- **Explicit peers**: Fallback for testing reliability

## Related Files

- `agents/commander/AGENTS.md` - MCP tools, coordination
- `agents/router/AGENTS.md` - Specialist selection
- `agents/specialists/*/AGENTS.md` - Domain expertise
- `compose.yaml` - Minecraft server config
- `logs/` - Per-agent logs (gitignored)
