# Orchestrator Agent Example

<!-- ARKAVO-CAPABILITY: orchestrator -->
> **Specs**: [11 scenarios](../../specs/arkavo-edge/orchestrator.spec.yaml)
> **Browse**: `cargo xtask capabilities orchestrator`
<!-- /ARKAVO-CAPABILITY -->

This example demonstrates an AI Orchestrator Agent that coordinates work across a mesh of specialized agents.

## Architecture

```
Human → [AI Orchestrator Agent] → [Specialist Agent 1]
                                → [Specialist Agent 2]
                                → [Specialist Agent N]
```

The orchestrator:
- Receives human requests via AG-UI chat interface
- Analyzes requests to determine required capabilities
- Discovers available specialist agents via mDNS
- Routes tasks to appropriate specialists using A2A protocol
- Monitors task execution and reports results

## Quick Start

1. Start some specialist agents in separate terminals:

```bash
# Terminal 1: Security specialist
cd /path/to/security-agent
arkavo agent run

# Terminal 2: Code review specialist
cd /path/to/code-review-agent
arkavo agent run
```

2. Start the orchestrator:

```bash
cd examples/orchestrator-agent
arkavo agent run
```

3. Open the AG-UI to interact:

```bash
arkavo ui
```

4. Chat with the orchestrator. It will route your requests to the appropriate specialists.

## Example Requests

- "Review the authentication code for security vulnerabilities"
  - Orchestrator routes to security specialist

- "Fix the performance issue in the database queries"
  - Orchestrator routes to database specialist

- "Create comprehensive tests for the payment module"
  - Orchestrator routes to testing specialist

## MCP Tools

The orchestrator uses these tools to coordinate the mesh:

| Tool | Purpose |
|------|---------|
| `list_agents` | Discover all agents on the mesh |
| `agent_query` | Find agents with specific capabilities |
| `send_task` | Delegate task to a specialist agent |
| `get_task_status` | Monitor task progress |

## Configuration

See `orchestrator-agent.swarmkit.yaml` for the orchestrator configuration:
the routing system prompt lives in the `orchestrator` role's `skill:identity`
instructions, and `runtime.listen: 0.0.0.0:8340` pins the fixed port used
above. The tool grants in the table above are not yet formalized as
`mcp_tools` in the kit (the original config never declared them either) —
they describe what the orchestrator is expected to call at runtime.
