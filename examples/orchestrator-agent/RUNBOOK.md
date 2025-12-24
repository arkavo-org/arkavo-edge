# Orchestrator Agent - RUNBOOK

## What This Example Demonstrates

This example demonstrates a **Central Orchestrator Pattern**:

1. **Task Routing**: Orchestrator analyzes requests and selects appropriate specialists
2. **Agent Discovery**: Uses mDNS to find available agents on the mesh
3. **Capability Matching**: Selects agents based on skills and purpose alignment
4. **Multi-Agent Coordination**: Decomposes complex tasks across specialists
5. **Result Aggregation**: Combines responses from multiple agents

The scenario: An intelligent coordinator that routes human requests to specialized agents.

## Prerequisites

```bash
# Build the binary and mesh tools
cargo build

# Verify port 8340 is free
lsof -i :8340 && echo "Port 8340 is in use!"

# Verify mesh tools are built
ls -la target/debug/arkavo-mesh-tools
```

## Step-by-Step Execution

### Step 1: Start Specialist Agents

The orchestrator needs specialist agents to route requests to. Start some from other examples:

```bash
# Terminal 1: Start a security specialist
cd examples/software-development-lifecycle/security
../../target/debug/arkavo agent run

# Terminal 2: Start a code review specialist
cd examples/software-development-lifecycle/code-review
../../target/debug/arkavo agent run
```

**What to watch for:**
- Each agent starts and listens on its configured port
- mDNS service registration messages
- Health check endpoints responding

### Step 2: Start the Orchestrator

```bash
cd examples/orchestrator-agent
../../target/debug/arkavo agent run
```

**What to watch for:**
- Orchestrator starts on port 8340
- mDNS discovery finds specialist agents
- "Registered peer" messages for discovered agents

**Expected output:**
```
Starting orchestrator agent...
Listening on 0.0.0.0:8340
mDNS: Discovered security agent at ...
mDNS: Discovered code-review agent at ...
```

### Step 3: Verify Agent Discovery

```bash
# Check mDNS service advertisements
dns-sd -B _a2a._tcp local.

# Check orchestrator is running
curl -s http://localhost:8340/health

# Check which agents are discovered
lsof -i -P | grep arkavo
```

**What to watch for:**
- Orchestrator and specialists all appear in mDNS
- Orchestrator health check returns success
- Multiple arkavo processes running

### Step 4: Test Task Routing

```bash
# Send a request via chat
timeout 60 ../../target/debug/arkavo chat --prompt "Review this code for security issues: def login(u, p): return db.query('SELECT * FROM users WHERE user=' + u)"
```

**What to watch for:**
- Orchestrator receives the request
- Orchestrator calls `list_agents` to discover specialists
- Orchestrator routes to security agent
- Response includes security analysis

### Step 5: Open the AG-UI (Optional)

```bash
../../target/debug/arkavo ui
```

**What to watch for:**
- Web interface opens in browser
- Orchestrator appears in agent grid
- Click on orchestrator to chat

### Step 6: Stop All Agents

```bash
# Stop all arkavo processes
pkill -f "arkavo agent"

# Verify cleanup
ps aux | grep "arkavo agent"
```

## Automated Validation

```bash
#!/bin/bash
# test_example.sh - Run this for automated validation

# Start a specialist in background
cd examples/software-development-lifecycle/security
../../target/debug/arkavo agent run &
SPECIALIST_PID=$!
sleep 3

# Start orchestrator in background
cd ../../orchestrator-agent
../../target/debug/arkavo agent run &
ORCHESTRATOR_PID=$!
sleep 5

# Verify orchestrator is healthy
if ! curl -sSf http://localhost:8340/health > /dev/null 2>&1; then
    echo "FAIL: Orchestrator not responding"
    kill $ORCHESTRATOR_PID $SPECIALIST_PID 2>/dev/null
    exit 1
fi

echo "PASS: Orchestrator is healthy"
kill $ORCHESTRATOR_PID $SPECIALIST_PID 2>/dev/null
```

## MCP Tools Used

| Tool | Purpose |
|------|---------|
| `list_agents` | Discover all agents on the mesh |
| `agent_query` | Find agents with specific capabilities |
| `send_task` | Delegate task to a specialist agent |
| `get_task_status` | Monitor task progress |

## Common Failure Modes

### Port already in use
**Symptom:** Orchestrator fails to start
**Fix:** Kill existing process
```bash
lsof -ti :8340 | xargs kill -9
```

### No specialist agents found
**Symptom:** Orchestrator can't route requests
**Fix:** Start specialist agents first, verify mDNS
```bash
dns-sd -B _a2a._tcp local.
```

### MCP tools not found
**Symptom:** Error about arkavo-mesh-tools
**Fix:** Build the mesh tools crate
```bash
cargo build -p arkavo-mesh-tools
```

### Agent discovery timeout
**Symptom:** Orchestrator starts but no peers found
**Fix:** Wait longer for mDNS discovery
```bash
# Allow 10-15 seconds for discovery
sleep 15
```

## Architecture Notes

- Orchestrator uses Claude Sonnet for intelligent routing
- Specialists can use any model (local or cloud)
- All agents use mDNS (`_a2a._tcp.local.`) for discovery
- Orchestrator delegates entire tasks, not just queries
- Results flow back through orchestrator to human
