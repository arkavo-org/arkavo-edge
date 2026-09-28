# Software Development Simple Demo

This example demonstrates sophisticated agent-to-agent communication using the Arkavo A2A (Agent-to-Agent) protocol. Three specialized agents collaborate on software development tasks, showcasing real-world multi-agent orchestration.

## Overview

The demo implements a software development workflow where:
- **Project Manager Agent** orchestrates tasks and coordinates between agents
- **Coding Agent** implements features and writes code
- **Testing Agent** validates code quality and generates tests

## Architecture

```
┌─────────────────────┐
│  Project Manager    │
│    (Port 8342)      │
│  - Task Planning    │
│  - Coordination     │
└──────────┬──────────┘
           │
    ┌──────┴──────┐
    │             │
    v             v
┌─────────┐   ┌─────────┐
│ Coding  │   │ Testing │
│ Agent   │◄──► Agent   │
│ (8343)  │   │ (8344)  │
└─────────┘   └─────────┘
```

## Prerequisites

1. Build the Arkavo binary:
```bash
cargo build
```

2. Ensure you have a local model or Ollama configured:
```bash
# For local models, arkavo will auto-detect GGUF files
arkavo model list

# Or configure Ollama if available
curl http://ollama:11434/api/tags
```

3. Install the AGUI dashboard (optional but recommended):
```bash
# The dashboard provides real-time monitoring
arkavo ui
```

## Quick Start

1. **Start all agents:**
```bash
cd examples/software-development-simple
./launch.sh
```

2. **Run the demo scenarios:**
```bash
./run_scenarios.sh all
```

3. **Monitor via AGUI dashboard:**
```bash
arkavo ui
# Open browser to http://localhost:3000
```

## Communication Patterns

### 1. Agent Discovery (mDNS)
Agents automatically discover each other using multicast DNS:
- Each agent broadcasts its capabilities
- Agents maintain a registry of available services
- Automatic failover and reconnection

### 2. Direct Messaging
Point-to-point communication between specific agents:
```json
{
  "method": "message/send",
  "params": {
    "message": {
      "from_agent": "project-manager",
      "to_agent": "coding-agent",
      "content": "Implement Calculator class",
      "message_type": "task_assignment"
    }
  }
}
```

### 3. Query-Response
Synchronous request-response pattern:
```json
{
  "method": "agent_query",
  "params": {
    "request": {
      "from_agent_id": "project-manager",
      "to_agent_id": "coding-agent",
      "query": "What is your current status?"
    }
  }
}
```

### 4. Broadcast
One-to-many capability announcements:
```json
{
  "method": "agent_broadcast",
  "params": {
    "broadcast": {
      "agent_id": "testing-agent",
      "broadcast_type": "capabilities",
      "capabilities": [...]
    }
  }
}
```

### 5. Streaming
Real-time updates via WebSocket:
```json
{
  "method": "message/stream",
  "params": {
    "task_id": "calc-001"
  }
}
```

## Demo Scenarios

### Scenario 1: Agent Discovery
Demonstrates how agents find and register with each other:
```bash
./run_scenarios.sh discovery
```

### Scenario 2: Direct Query
Shows agent-to-agent status queries:
```bash
./run_scenarios.sh query
```

### Scenario 3: Task Assignment
Complete workflow from task assignment to completion:
```bash
./run_scenarios.sh task
```

### Scenario 4: Chat Session
Multi-agent interactive planning session:
```bash
./run_scenarios.sh chat
```

### Scenario 5: Concurrent Tasks
Parallel task execution across multiple agents:
```bash
./run_scenarios.sh concurrent
```

## Agent Endpoints

### HTTP/REST Endpoints
- Project Manager: http://localhost:8342
- Coding Agent: http://localhost:8343
- Testing Agent: http://localhost:8344

### WebSocket Endpoints
- Project Manager: ws://localhost:8342/ws
- Coding Agent: ws://localhost:8343/ws
- Testing Agent: ws://localhost:8344/ws

### Health Check
```bash
curl http://localhost:8342/.well-known/agent.json
curl http://localhost:8343/.well-known/agent.json
curl http://localhost:8344/.well-known/agent.json
```

## Project Structure

```
software-development-simple/
├── software-development-simple.swarmkit.yaml  # All 3 agent roles (single kit)
├── project-manager/
│   └── workspace/          # Working directory
├── coding-agent/
│   └── workspace/          # Code output directory
├── testing-agent/
│   └── test-results/       # Test reports
├── launch.sh               # Start/stop all agents
├── run_scenarios.sh        # Run demo scenarios
├── logs/                   # Agent log files
└── README.md              # This file
```

## Monitoring and Debugging

### View Agent Logs
```bash
# Real-time log monitoring
tail -f logs/*.log

# Individual agent logs
tail -f logs/project-manager.log
tail -f logs/coding-agent.log
tail -f logs/testing-agent.log
```

### Check Agent Status
```bash
./launch.sh status
```

### AGUI Dashboard
The Arkavo UI provides real-time monitoring:
1. Agent health status
2. Message flow visualization
3. Task progress tracking
4. Performance metrics

Access at: http://localhost:3000

## Managing Agents

### Start Agents
```bash
./launch.sh start
```

### Stop Agents
```bash
./launch.sh stop
```

### Restart Agents
```bash
./launch.sh restart
```

### View Log Locations
```bash
./launch.sh logs
```

## Customization

### Modify Agent Behavior
Edit the corresponding role in `software-development-simple.swarmkit.yaml` to:
- Change the LLM model (`agent_provisioning.model`)
- Adjust listening ports (pass a different `-p` in `launch.sh`)
- Add MCP tool grants (`mcp_tools`)
- Configure capabilities (the role's `skill:identity` instructions)

### Add New Agents
1. Add a new entry under `roles:` in `software-development-simple.swarmkit.yaml`
2. Configure a unique port and its `skill:identity` capabilities
3. Update `launch.sh` to include the new agent
4. Add communication patterns in run_scenarios.sh

### Change Models
Update the role's `agent_provisioning.model` in
`software-development-simple.swarmkit.yaml`:
```yaml
agent_provisioning:
  model:
    family: "ministral"
    size: "3B"
    backend: "llama.cpp"
```
(`project-manager` and `testing-agent` shipped with `gemma-3-270m`, which
predates this repo's local-edge-model vocabulary — see the kit file's header
comment. `coding-agent` ran with no model hint, i.e. router default.)

## Troubleshooting

### Agents Won't Start
- Check if ports 8342-8344 are available
- Verify Ollama is running: `curl http://127.0.0.1:11434/api/tags`
- Check binary exists: `ls ../../target/debug/arkavo`

### Communication Failures
- Verify all agents are healthy: `./launch.sh status`
- Check `runtime.mdns: true` is set in `software-development-simple.swarmkit.yaml`
- Review logs for connection errors

### Performance Issues
- Use smaller models for faster response
- Adjust rate limiting in agent configurations
- Monitor resource usage via AGUI dashboard

## Technical Details

### A2A Protocol
The Agent-to-Agent protocol uses:
- JSON-RPC 2.0 over WebSocket/HTTP
- mDNS for service discovery
- Task-based message routing
- Capability-based agent selection

### Security
- Rate limiting per agent
- Authentication support (optional)
- TLS encryption (configurable)
- API key management

### Scalability
- Supports horizontal scaling
- Load balancing via orchestrator
- Async task execution
- Message queue integration ready

## Learn More

- [Arkavo Documentation](../../README.md)
- [A2A Protocol Specification](../../docs/a2a-protocol.md)
- [MCP Integration Guide](../../docs/mcp-integration.md)
- [AGUI Dashboard Guide](../../crates/arkavo-agui/README.md)

## Contributing

To extend this demo:
1. Add new scenarios in run_scenarios.sh
2. Implement additional agent types
3. Create more complex workflows
4. Add integration tests

## License

This example is part of the Arkavo project and follows the same license terms.