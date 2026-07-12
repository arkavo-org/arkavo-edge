# Software Development Lifecycle - RUNBOOK

## What This Example Demonstrates

This example demonstrates **Multi-Agent Knowledge Sharing** for software development:

1. **Task Decomposition**: Orchestrator breaks complex requests into subtasks
2. **Domain Specialization**: 12 agents with focused expertise areas
3. **Agent-to-Agent Communication**: Direct queries between specialists
4. **mDNS Discovery**: Zero-configuration agent networking
5. **Collaborative Problem Solving**: Agents consult each other for cross-domain issues

The scenario: A mesh of specialized agents that collaborate to review, analyze, and improve software projects.

## Prerequisites

```bash
# Build the binary
cargo build

# Verify ports are free (agents use 8342-8353)
for port in {8342..8353}; do
    lsof -i :$port && echo "Port $port is in use!"
done

# Optional: Install MCP servers for enhanced capabilities
npm install -g @modelcontextprotocol/server-filesystem
npm install -g @cyanheads/git-mcp-server
```

## Step-by-Step Execution

### Step 1: Launch the Agent Mesh

```bash
cd examples/software-development-lifecycle
./launch.sh
```

**What to watch for:**
- All 12 agents start successfully (ports 8342-8353)
- Health checks pass for each agent (green checkmarks)
- mDNS discovery messages in logs
- Warning about optional MCP servers is OK (agents work without them)

**Expected output:**
```
╔═══════════════════════════════════════════════════════════════╗
║     ARKAVO - Multi-Agent Software Development System          ║
║                   11 Specialized Agents                       ║
╚═══════════════════════════════════════════════════════════════╝

[OK] Arkavo binary found
[OK] npx available (for MCP servers)
[INFO] Starting orchestrator (port 8342)...
[INFO] Starting security (port 8343)...
[INFO] Starting code-review (port 8344)...
...
[INFO] Waiting for agents to initialize...

Agent Status:
=============
[OK] orchestrator (port 8342)
[OK] security (port 8343)
...
```

### Step 2: Verify Agent Discovery

```bash
# Check mDNS service advertisements
dns-sd -B _a2a._tcp local.

# Check arkavo processes are running
ps aux | grep "arkavo agent"

# Check agents are listening
lsof -i -P | grep arkavo
```

**What to watch for:**
- Services registered as `_a2a._tcp.local.`
- Each agent has its own arkavo process
- Agents listening on dynamic ports (discovered via mDNS)

### Step 3: Test Agent Interaction

```bash
# Query a specific agent directly
timeout 60 ../../target/debug/arkavo chat --prompt "Analyze this code for security issues: def login(user, pass): query = 'SELECT * FROM users WHERE user=' + user"
```

**What to watch for:**
- Orchestrator receives the request
- Orchestrator identifies need for security analysis
- Security agent provides vulnerability assessment
- Response mentions SQL injection risk

### Step 4: Monitor Agent Collaboration

```bash
# Watch logs in real-time
tail -f logs/*.log

# Check specific agent logs
tail -50 logs/orchestrator.log
tail -50 logs/security.log
```

**What to watch for:**
- `agent_query` RPC calls between agents
- Capability broadcasts
- Task decomposition in orchestrator logs
- Domain-specific analysis in specialist logs

### Step 5: Check Status

```bash
./launch.sh status
```

**What to watch for:**
- All 12 agents responding to health checks
- No "not responding" warnings

### Step 6: Stop the Mesh

```bash
./launch.sh stop
```

**What to watch for:**
- All agent processes terminated
- No orphan processes remaining

**Verify cleanup:**
```bash
ps aux | grep "arkavo agent"
lsof -i :8342 -i :8343 -i :8344
# Should return empty
```

## Automated Validation

```bash
#!/bin/bash
# test_example.sh - Run this for automated validation

./launch.sh start
sleep 15

# Verify orchestrator is healthy
if ! curl -sSf http://localhost:8342/.well-known/agent.json > /dev/null 2>&1; then
    echo "FAIL: Orchestrator not responding"
    ./launch.sh stop
    exit 1
fi

# Verify at least 5 agents are running
AGENT_COUNT=$(ps aux | grep "arkavo agent" | grep -v grep | wc -l)
if [ "$AGENT_COUNT" -lt 5 ]; then
    echo "FAIL: Only $AGENT_COUNT agents running (expected 12)"
    ./launch.sh stop
    exit 1
fi

echo "PASS: Agent mesh is healthy ($AGENT_COUNT agents running)"
./launch.sh stop
```

## Agent Roles

| Agent | Port | Expertise |
|-------|------|-----------|
| Orchestrator | 8342 | Task decomposition, agent coordination |
| Security | 8343 | Vulnerability analysis, auth review |
| Code Review | 8344 | Code quality, refactoring |
| Database | 8345 | SQL optimization, schema design |
| Testing | 8346 | Test generation, coverage |
| Documentation | 8347 | API docs, README generation |
| Performance | 8348 | Profiling, optimization |
| DevOps | 8349 | CI/CD, deployment |
| Frontend | 8350 | UI/UX, accessibility |
| Architecture | 8351 | System design, scalability |
| Data Science | 8352 | ML models, data analysis |
| Debug | 8353 | Error analysis, self-healing |

## Common Failure Modes

### Port already in use
**Symptom:** Agent fails to start, error about binding
**Fix:** Kill existing process or choose different ports
```bash
lsof -ti :8342 | xargs kill -9
```

### mDNS discovery timeout
**Symptom:** Agents start but can't find each other
**Fix:** Wait longer (10-15 seconds) or verify mDNS is enabled
```bash
# Check if mDNS service is running (macOS)
dns-sd -B _a2a._tcp local.
```

### MCP server not found
**Symptom:** Warning about MCP filesystem server
**Fix:** This is OK - agents work without external MCP servers
```bash
# Optional: Install MCP servers
npm install -g @modelcontextprotocol/server-filesystem
```

### Out of memory
**Symptom:** Agents crash randomly, OOM killer messages
**Fix:** Run fewer agents or increase memory allocation
```bash
# Start only core agents
./launch.sh  # Then stop non-essential agents
```

## Example Workflow

When you ask: "Review my Python web app for security issues"

1. **Orchestrator** receives request, identifies need for security analysis
2. **Orchestrator** queries **Security Agent**: "Analyze for vulnerabilities"
3. **Security Agent** finds SQL injection risk
4. **Security Agent** queries **Code Review Agent**: "Is this parameterized?"
5. **Code Review Agent** confirms vulnerability pattern
6. **Security Agent** might query **Database Agent** for proper SQL patterns
7. **Orchestrator** aggregates findings and presents report

## Architecture Notes

- Agents use mDNS (`_a2a._tcp.local.`) for discovery
- Each agent is a role in `software-development-lifecycle.swarmkit.yaml`, selected at launch via `-n <role-id>`
- Ports are passed at launch via `-p <port>`; mDNS handles actual discovery
- No central database - knowledge stays with specialists
- Agents query each other on-demand for cross-domain expertise
