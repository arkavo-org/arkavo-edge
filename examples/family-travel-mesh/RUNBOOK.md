# Family Travel Mesh - RUNBOOK

## What This Example Demonstrates

This example demonstrates **HRM-Style Orchestration** (Hierarchical Reasoning Model):

1. **Slow Loop (Conductor)**: Task decomposition into subtasks
2. **Medium Loop (Router)**: Thompson Sampling agent selection
3. **Fast Loop (Specialists)**: Domain-expert execution with burst contracts
4. **Verification (Critic)**: Policy enforcement before approval
5. **Memory Service**: Tiered context management (STM, Task, LTM)

The scenario: Planning a Friday afternoon in Las Vegas for a family with twin toddlers (age 3).

## Prerequisites

```bash
# Build the binary
cargo build

# Verify ports are free
for port in 8401 8402 8403 8404 8410 8411 8412; do
    lsof -i :$port && echo "Port $port is in use!"
done
```

## Step-by-Step Execution

### Step 1: Launch the Mesh

```bash
cd examples/family-travel-mesh
./launch_mesh.sh
```

**What to watch for:**
- All 7 agents start successfully (ports 8401-8404, 8410-8412)
- Health checks pass for each agent (green checkmarks)
- mDNS discovery completion (5-second wait after last agent)

**Expected output:**
```
[INFO] Starting Memory Service (8404)...
[INFO] Starting Critic (8403)...
[INFO] Starting Specialists...
[INFO]   - vegas-guide (8410)
[INFO]   - family-activities (8411)
[INFO]   - budget-optimizer (8412)
[INFO] Starting Router (8402)...
[INFO] Starting Conductor (8401)...
[INFO] Mesh startup complete. Waiting for discovery...
[OK] All agents healthy
```

### Step 2: Run the Demo

```bash
./guided_demo.sh
```

**What to watch for:**
- Task sent to conductor message
- Response contains family-friendly Vegas activities (NOT casinos)
- "RESPONSE APPROVED" green box (policy passed)
- If you see RED box, the Critic detected policy violations

**Expected behavior:**
1. Prompt sent to mesh via conductor
2. Conductor decomposes task
3. Router selects specialist (may try vegas-guide first)
4. If vegas-guide selected: Critic VETOES casino recommendations
5. Router re-routes to family-activities
6. family-activities provides safe recommendations
7. Critic APPROVES
8. Response returned

### Step 3: Verify Mesh Discovery

```bash
# Check router logs for peer registration
grep -i "peer" logs/router.log

# Check conductor logs for task flow
tail -50 logs/conductor.log
```

**What to watch for:**
- "Registered peer" messages in router log
- Task ID creation in conductor log
- Subtask decomposition messages

### Step 4: Clean Shutdown

```bash
./stop_mesh.sh
```

**What to watch for:**
- All agent processes terminated
- No orphan processes on ports 8401-8412

**Verify cleanup:**
```bash
lsof -i :8401 -i :8402 -i :8403 -i :8404 -i :8410 -i :8411 -i :8412
# Should return empty
```

## Automated Validation

```bash
#!/bin/bash
# test_example.sh - Run this for automated validation

./launch_mesh.sh
sleep 15

# Verify conductor is healthy
if ! curl -sSf http://localhost:8401/health > /dev/null; then
    echo "FAIL: Conductor not responding"
    ./stop_mesh.sh
    exit 1
fi

# Verify mesh discovery
if ! grep -q "peer" logs/router.log 2>/dev/null; then
    echo "WARN: No peer registration in logs (may be too early)"
fi

echo "PASS: Mesh is healthy"
./stop_mesh.sh
```

## Common Failure Modes

### Port already in use
**Symptom:** Agent fails to start, error about binding
**Fix:** Kill existing process or choose different ports
```bash
lsof -ti :8401 | xargs kill -9
```

### mDNS discovery timeout
**Symptom:** Agents start but can't find each other
**Fix:** Wait longer (10-15 seconds) or verify mDNS is enabled
```bash
# Check if mDNS service is running (macOS)
dns-sd -B _a2a._tcp local.
```

### Policy violation not detected
**Symptom:** Vegas-guide recommendations get through without veto
**Fix:** Verify Critic is running and policies are loaded
```bash
cat logs/critic.log | grep -i "policy\|veto"
```

### Out of memory (CI runners)
**Symptom:** Agents crash randomly, OOM killer messages
**Fix:** Run fewer agents or increase memory allocation

## Architecture Notes

- **Vegas-guide is intentionally rigged** to always recommend casinos
- This triggers the Critic's policy enforcement
- Router learns via Thompson Sampling to prefer family-activities
- See `agents/specialists/vegas-guide/AGENTS.md` for details
