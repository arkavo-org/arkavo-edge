# Core Concepts

This guide explains the fundamental concepts behind Arkavo's agent system. Understanding these will help you build effective multi-agent applications.

## Table of Contents

- [Agent Architecture](#agent-architecture)
- [SwarmKit Configuration](#swarmkit-configuration)
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

## SwarmKit Configuration

Every agent or mesh is configured by a **SwarmKit manifest** — a `*.swarmkit.yaml` file that declares one or more **roles**, each with its own identity, model provisioning, and tool grants. A single agent is simply a one-role kit; a mesh is a multi-role kit with a `coordination` block wiring the roles together. This section covers the concepts you need to read and edit a kit; see [docs/SWARMKIT.md](../docs/SWARMKIT.md) for the full manifest schema.

### Minimal Configuration

Scaffold a starting-point kit with `arkavo kit init my-agent` (writes `.arkavo/my-agent.swarmkit.yaml`), then edit it. An abbreviated single-role kit — see `01-hello-world/hello-agent.swarmkit.yaml` for the complete file:

```yaml
spec_version: "1.0.0"
kit:
  id: ""            # leave empty while editing; `arkavo kit validate` prints the computed BLAKE3 id to paste here
  name: "hello-agent"
  version: "0.1.0"

objective:
  goal: "Introduce yourself and answer basic questions helpfully"

roles:
  - id: "agent"
    role_type: "operator"
    agent_provisioning:
      model:
        family: "ministral"
        size: "3B"
        backend: "llama.cpp"
    skills:
      - id: "skill:identity"
        source: "inline"
        payload:
          instructions: >-
            You are a friendly agent that introduces itself and
            answers basic questions.

runtime:
  mode: orchestrator
  mdns: true
  local_dev: true
```

### Configuration Fields

| Field | Required | Description |
|-------|----------|-------------|
| `roles[].id` | Yes | Role identifier (the agent's name) |
| `objective.goal` | Yes | The kit's overall purpose; the primary role's identity skill carries the same purpose in more detail |
| `roles[].skills[].payload.instructions` | Yes | The role's system prompt / identity — what it does, shown to other agents |
| `roles[].agent_provisioning.model` | No | LLM family/size/backend to provision (see Model Providers below); omit to accept the router default |
| `runtime.listen` | No | Bind address (default: `127.0.0.1` on a dynamic port). Set it, for example to `0.0.0.0:8342`, for agents on other devices to reach this one |
| `runtime.mdns` | No | Enable mDNS discovery (default: true) |
| `runtime.mcp_servers` + `roles[].mcp_tools` | No | MCP tool servers to connect, and per-role grants against them |

### Model Providers

`agent_provisioning.model` names the model a role runs on, local or cloud. A locally-hosted edge model is a `family`/`size` pair:

```
family: ministral, size: 3B / 8B     # Local Ministral (edge-optimized)
family: gemma,     size: E2B / E4B / 12B   # Local Gemma 4
family: qwen,      size: 0.8B / 9B / 27B   # Local Qwen 3.5
```

Any other model the router knows, cloud models included, is its router id as the `family` with no `size`:

```
family: gpt-6-astra
family: kimi-k2.5
family: gemini-3.5-flash
```

Naming a cloud model in the kit counts as consent to use it under the kit's `runtime.cloud_policy`. Omit `model` to accept the router default. `arkavo agent` refuses to start a role whose model the router does not know, and `arkavo kit validate` fails on it.

API keys for cloud providers are read from the environment and never written in the kit; see the READMEs of [code-agent-claude](code-agent-claude/README.md) and [code-agent-gemini](code-agent-gemini/README.md).

### Validating and Running

```bash
arkavo kit validate <path/to/kit.swarmkit.yaml>
arkavo agent -c <path/to/kit.swarmkit.yaml> [-n <role-id>] [-p <port>]
```

`-n` selects a role from a multi-role kit (default: the first role); `-p` overrides the listen port. Converting a legacy AGENTS.md into a starting-point kit: `arkavo kit migrate-from-agents-md --in <file> --out <kit>` (best-effort — hand-finish anything it can't map).

---

## mDNS Discovery

Agents discover each other automatically using **mDNS** (multicast DNS), also known as Bonjour/Avahi. Between agents on the same machine this needs no configuration.

An agent listens on `127.0.0.1` by default, so agents on other devices cannot reach it. To form a mesh across devices, set `runtime.listen` in the kit of every agent that must be reachable:

```yaml
runtime:
  listen: "0.0.0.0:8342"
```

The agent's RPC endpoint does not authenticate callers yet, so bind it to other interfaces only on a trusted network.

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

### Beyond the Local Network

mDNS works on a single local network. A kit has no setting for static peers, so agents on different networks do not find each other through kit configuration.

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

In the kit's `runtime.mcp_servers` (the process to launch) plus a per-role `mcp_tools` grant (who may call it):

```yaml
runtime:
  mcp_servers:
    - name: code-search
      command: /usr/local/bin/mcp-code-search
      args: ["--repo", "/path/to/repo"]

    - name: web-browser
      command: npx
      args: ["-y", "@anthropic/mcp-browser"]

roles:
  - id: "agent"
    mcp_tools:
      - server: "code-search"
        auth: "delegated"
        tools: []
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

In the kit's `runtime.preflight.policies`:

```yaml
runtime:
  preflight:
    policies:
      - id: "block_pii"
        features:
          - "InputContainsPII"
        action: block
        description: "Blocks SSN, credit card numbers, and email addresses"
        enabled: true
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
