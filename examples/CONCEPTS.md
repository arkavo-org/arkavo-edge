# Core Concepts

This guide explains the fundamental concepts behind Arkavo's agent system. Understanding these will help you build effective multi-agent applications.

## Table of Contents

- [Agent Architecture](#agent-architecture)
- [AGENTS.md Configuration](#agentsmd-configuration)
- [mDNS Discovery](#mdns-discovery)
- [A2A Protocol](#a2a-protocol)
- [HRM Pattern](#hrm-pattern)
- [Thompson Sampling](#thompson-sampling)
- [Gossip Learning](#gossip-learning)
- [MCP Integration](#mcp-integration)
- [Preflight Policies](#preflight-policies)

---

## Agent Architecture

An **agent** is an autonomous AI process that:
1. Has a specific **purpose** (what it's good at)
2. Uses an **LLM** for reasoning (local or cloud)
3. **Discovers** other agents via mDNS
4. **Communicates** via A2A protocol
5. **Exposes tools** via MCP

### Agent Lifecycle

```
Start → Bind Port → Advertise via mDNS → Accept Connections → Process Tasks → Shutdown
```

### Single Agent vs Mesh

| Mode | Description | Use Case |
|------|-------------|----------|
| Single Agent | One agent, direct interaction | Simple tasks, prototyping |
| Mesh | Multiple agents, collaborate | Complex workflows, specialization |

---

## AGENTS.md Configuration

Every agent is configured via an `AGENTS.md` file in its directory. This markdown file defines the agent's identity and behavior.

### Minimal Configuration

```markdown
## my-agent
purpose: "Answer questions about code"
model: ministral-3b
```

### Full Configuration

```markdown
## code-reviewer
purpose: |
  Review code for bugs, security issues, and style.
  Provide actionable feedback with line references.

model: gemini-2.0-flash
listen: 0.0.0.0:8342
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8343"

mcp_servers:
  - name: code-search
    command: /path/to/mcp-code-search
    args: ["--repo", "."]
```

### Configuration Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes (from header) | Agent identifier |
| `purpose` | Yes | What this agent does (shown to other agents) |
| `model` | Yes | LLM to use (see Model Providers below) |
| `listen` | No | Bind address (default: `0.0.0.0:0` for dynamic) |
| `mdns` | No | Enable mDNS discovery (default: true) |
| `a2a.enabled` | No | Enable A2A protocol (default: true) |
| `a2a.peers` | No | Static peer list (optional with mDNS) |
| `mcp_servers` | No | MCP tool servers to connect |

### Model Providers

```
ministral-3b          # Local Ministral (edge-optimized)
gemma-3-270m          # Local Gemma (very small)
ollama://host/model   # Ollama server
claude-3-5-sonnet     # Anthropic Claude (requires ANTHROPIC_API_KEY)
gemini-2.0-flash      # Google Gemini (requires GEMINI_API_KEY)
```

---

## mDNS Discovery

Agents discover each other automatically using **mDNS** (multicast DNS), also known as Bonjour/Avahi. This enables zero-configuration networking.

### How It Works

1. Agent starts and binds to a port
2. Agent advertises itself: `agent-name._a2a._tcp.local.`
3. Other agents browse for `_a2a._tcp.local.`
4. Discovery provides: name, IP, port, purpose

### Service Type

All Arkavo agents use the service type: `_a2a._tcp.local.`

### Debugging Discovery

```bash
# List all discovered agents (macOS)
dns-sd -B _a2a._tcp local.

# Get details for a specific agent
dns-sd -L "agent-name" _a2a._tcp local.

# Linux alternative
avahi-browse -art | grep a2a
```

### When to Use Static Peers

mDNS works on local networks. For cross-network scenarios, use static peers:

```yaml
a2a:
  peers:
    - "http://192.168.1.100:8342"
    - "http://server.example.com:8342"
```

---

## A2A Protocol

**A2A** (Agent-to-Agent) is the communication protocol between agents. It uses JSON-RPC 2.0 over HTTP/WebSocket.

### Message Types

| Method | Direction | Description |
|--------|-----------|-------------|
| `agent.card` | Request | Get agent's identity and capabilities |
| `task.submit` | Request | Send a task to an agent |
| `task.status` | Request | Check task progress |
| `task.cancel` | Request | Cancel a running task |
| `message.stream` | Stream | Real-time response streaming |

### Example: Task Submission

```json
{
  "jsonrpc": "2.0",
  "method": "task.submit",
  "params": {
    "task": "Review this code for security issues",
    "context": { "file": "auth.py", "content": "..." }
  },
  "id": 1
}
```

### Example: Agent Card Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "name": "security-reviewer",
    "purpose": "Review code for security vulnerabilities",
    "model": "claude-3-5-sonnet",
    "capabilities": ["code-review", "security-audit"]
  },
  "id": 1
}
```

### Transport

- **HTTP**: One-shot request/response
- **WebSocket**: Streaming responses, bidirectional

---

## HRM Pattern

**HRM** (Hierarchical Reasoning Model) is an orchestration pattern for complex multi-agent tasks. It splits reasoning into three loops operating at different speeds.

### The Three Loops

```
┌─────────────────────────────────────────────────────────┐
│  SLOW LOOP (Conductor)                                  │
│  - Task decomposition                                   │
│  - High-level planning                                  │
│  - Minutes between decisions                            │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│  MEDIUM LOOP (Router)                                   │
│  - Agent selection                                      │
│  - Load balancing                                       │
│  - Seconds between decisions                            │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│  FAST LOOP (Specialists)                                │
│  - Domain expertise                                     │
│  - Task execution                                       │
│  - Milliseconds response                                │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│  VERIFICATION (Critic)                                  │
│  - Policy enforcement                                   │
│  - Quality validation                                   │
│  - Feedback to Router                                   │
└─────────────────────────────────────────────────────────┘
```

### Component Roles

| Component | Role | Example |
|-----------|------|---------|
| **Conductor** | Breaks complex tasks into subtasks | "Plan Vegas trip" → 3 subtasks |
| **Router** | Picks the best agent for each subtask | Uses Thompson Sampling |
| **Specialists** | Execute domain-specific work | vegas-guide, budget-optimizer |
| **Critic** | Validates outputs against policies | Family-safe content check |

### When to Use HRM

- Complex tasks requiring multiple specialists
- Tasks with quality/policy requirements
- When you need to optimize agent selection over time

### Example: Family Travel Mesh

See `04-advanced-patterns/family-travel-mesh/` for a complete HRM implementation.

---

## Thompson Sampling

**Thompson Sampling** is a probabilistic algorithm the Router uses to select agents. It balances exploration (trying new agents) with exploitation (using proven agents).

### How It Works

1. Each agent has a **success rate** distribution (Beta distribution)
2. Router **samples** from each agent's distribution
3. Agent with **highest sample** is selected
4. **Outcome updates** the distribution (success/failure)

### Benefits

| Benefit | Description |
|---------|-------------|
| **Cold Start** | New agents get chances to prove themselves |
| **Adaptation** | Selection improves as data accumulates |
| **Exploration** | Occasionally tries underused agents |
| **No Tuning** | Works well without hyperparameter adjustment |

### Example

```
Agent A: 8 successes, 2 failures → Beta(9, 3)
Agent B: 2 successes, 1 failure  → Beta(3, 2)
Agent C: 0 successes, 0 failures → Beta(1, 1) ← New agent

Sample: A=0.72, B=0.58, C=0.81 → Select C (exploring new agent)
```

---

## Gossip Learning

**Gossip Learning** enables agents to share knowledge peer-to-peer without a central coordinator. When one agent learns something, it propagates to others.

### How It Works

1. Agent A encounters a hazard and learns a lesson
2. Agent A **broadcasts** the lesson to known peers
3. Each peer **stores** and **re-broadcasts** to their peers
4. Eventually, all agents have the lesson

### Message Format

```json
{
  "type": "lesson",
  "source": "rover-alpha",
  "content": {
    "condition": "sector_4_black_ice",
    "action": "reduce_speed",
    "confidence": 0.95
  },
  "ttl": 3
}
```

### TTL (Time-to-Live)

- Prevents infinite propagation
- Decrements on each hop
- Message dies when TTL reaches 0

### Use Cases

- Fleet learning (vehicles sharing road conditions)
- Distributed caching (agents sharing query results)
- Consensus building (agents voting on decisions)

### Example: Fleet Immunity

See `04-advanced-patterns/fleet-immunity/` for a complete gossip learning implementation.

---

## MCP Integration

**MCP** (Model Context Protocol) allows agents to use external tools. Tools are exposed by MCP servers that communicate via stdio.

### Architecture

```
┌─────────────┐    stdio    ┌─────────────┐
│   Agent     │◄───────────►│ MCP Server  │
│ (arkavo)    │             │ (tool host) │
└─────────────┘             └─────────────┘
```

### Configuring MCP Servers

In AGENTS.md:

```yaml
mcp_servers:
  - name: code-search
    command: /usr/local/bin/mcp-code-search
    args: ["--repo", "/path/to/repo"]

  - name: web-browser
    command: npx
    args: ["-y", "@anthropic/mcp-browser"]
```

### Available Tool Servers

| Server | Description |
|--------|-------------|
| `mcp-code-search` | Search code with ripgrep |
| `mcp-git` | Git operations |
| `mcp-filesystem` | File read/write |
| `mcp-browser` | Web browsing |
| Custom | Build your own in any language |

### Building Custom MCP Servers

MCP servers are simple programs that:
1. Read JSON-RPC requests from stdin
2. Write JSON-RPC responses to stdout
3. Advertise available tools via `tools/list`

See the MCP specification for details.

---

## Preflight Policies

**Preflight Policies** allow agents to validate inputs before processing. This enables content moderation, access control, and safety checks.

### How It Works

1. Request arrives at agent
2. **Preflight check** runs against policy rules
3. If **denied**, request is rejected with reason
4. If **allowed**, request proceeds to LLM

### Policy Configuration

In AGENTS.md:

```yaml
preflight:
  enabled: true
  policies:
    - name: no-harmful-content
      deny:
        - pattern: "how to (hack|exploit|attack)"
          reason: "Security policy violation"
    - name: rate-limit
      max_requests_per_minute: 60
```

### Policy Types

| Type | Description |
|------|-------------|
| **Pattern Match** | Regex-based content filtering |
| **Rate Limit** | Request throttling |
| **Size Limit** | Max input/output size |
| **Allow List** | Only permit specific sources |

### Example: Secure Agent

See `02-single-agent/secure-agent/` for a complete preflight policy implementation.

---

## Next Steps

Now that you understand the core concepts:

1. **Start simple**: Try `01-hello-world/` for your first agent
2. **Add collaboration**: Try `03-multi-agent-basics/` for multi-agent patterns
3. **Go advanced**: Try `04-advanced-patterns/` for HRM and gossip learning
4. **Build production**: Study `05-production/` for full system examples

See the main [README.md](README.md) for the complete learning path.
