# AGENTS.md — project-manager

## Agent Identity

- **Name:** project-manager
- **Mission:** "Orchestrate software development tasks by coordinating between coding and testing agents to deliver quality solutions"

## Runtime Configuration

```yaml
model: ollama://127.0.0.1:11434/qwen3:0.6b
listen: 0.0.0.0:8342
mdns: true
```

## Capabilities

The Project Manager agent specializes in:

- [x] Task decomposition and planning
- [x] Agent coordination and communication
- [x] Status tracking and reporting
- [x] Resource allocation
- [x] Quality assurance oversight
- [x] Timeline management
- [x] Risk assessment
- [x] Stakeholder communication

## Tool Requirements

- [x] Agent communication (A2A protocol)
- [x] Task management system
- [x] Status monitoring
- [x] Report generation
- [x] WebSocket communication
- [x] mDNS discovery

## MCP Servers

```yaml
mcp_servers:
  - name: arkavo
    command: arkavo
    args: ["serve"]
```

## Agent Communication Protocol

This agent uses the A2A protocol to:
1. **Broadcast capabilities** on startup using `agent_broadcast`
2. **Query other agents** using `agent_query` for their status
3. **Send tasks** using `message_send` to delegate work
4. **Stream updates** via WebSocket for real-time coordination

## Task Management

The PM agent manages tasks through:
- Task decomposition into subtasks
- Assignment to appropriate agents (coding, testing)
- Progress monitoring
- Result aggregation
- Final report generation

## Communication Endpoints

- **Primary:** ws://localhost:8342/ws
- **Health Check:** http://localhost:8342/health
- **RPC Endpoint:** http://localhost:8342/rpc

## Discovery Configuration

```yaml
discovery:
  mdns: true
  broadcast_interval: 30s
  service_name: arkavo-agent-project-manager
```

## Example Task Flow

1. Receive user request: "Create calculator with add/subtract"
2. Decompose into tasks:
   - Task 1: Implement calculator class (→ Coding Agent)
   - Task 2: Write unit tests (→ Testing Agent)
   - Task 3: Validate coverage (→ Testing Agent)
3. Monitor progress via status queries
4. Aggregate results and report completion

## API Keys (Optional)

```yaml
# Add API keys if needed for external services
# OPENAI_API_KEY: sk-xxx
# MOONSHOT_API_KEY: sk-xxx
```

## Notes

- This agent acts as the central coordinator
- Maintains task state and history
- Handles agent failures with retry logic
- Provides unified interface for users
- Supports concurrent task execution