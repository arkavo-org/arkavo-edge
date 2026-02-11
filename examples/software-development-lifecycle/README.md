# Arkavo Multi-Agent Knowledge Sharing System

<!-- ARKAVO-CAPABILITY: orchestrator -->
> **Specs**: [11 scenarios](../../specs/arkavo-edge/orchestrator.spec.yaml)
> **Browse**: `cargo xtask capabilities orchestrator`
<!-- /ARKAVO-CAPABILITY -->

This directory contains 11 hyper-specialized AI agents that collaborate through a mesh network to share knowledge and solve complex tasks.

## Architecture

The system uses a hybrid orchestrator-mesh architecture:
- **Orchestrator Agent**: Decomposes tasks and coordinates other agents
- **Specialized Agents**: Domain experts that can be queried directly or through the orchestrator
- **Mesh Communication**: Agents can query each other directly for specific knowledge
- **Shared Memory**: Agent conversations are stored in arkavo-memory for context preservation

## Agents

1. **Orchestrator** (port 8342)
   - Task decomposition
   - Agent coordination
   - Result aggregation

2. **Security** (port 8343)
   - Vulnerability analysis
   - Security best practices
   - Authentication/authorization review

3. **Code Review** (port 8344)
   - Code quality analysis
   - Pattern detection
   - Refactoring suggestions

4. **Database** (port 8345)
   - SQL optimization
   - Schema design
   - Index recommendations

5. **Testing** (port 8346)
   - Test generation
   - Coverage analysis
   - Test strategy

6. **Documentation** (port 8347)
   - API documentation
   - README generation
   - Code comments

7. **Performance** (port 8348)
   - Performance profiling
   - Optimization strategies
   - Bottleneck detection

8. **DevOps** (port 8349)
   - CI/CD design
   - Deployment automation
   - Infrastructure as code

9. **Frontend** (port 8350)
   - UI/UX analysis
   - Accessibility compliance
   - Responsive design

10. **Architecture** (port 8351)
    - System design
    - Scalability patterns
    - Microservices architecture

11. **Data Science** (port 8352)
    - ML model selection
    - Data analysis
    - Feature engineering

12. **Debug** (port 8353)
    - Session analysis and replay
    - Error pattern detection
    - Performance profiling
    - Self-healing recommendations

## Running the System

### Start All Agents
```bash
./launch_multi_agent_system.sh
```

### Start Individual Agent
```bash
cd examples/software-development-lifecycle/security
arkavo agent run
```

### Test Multi-Agent Collaboration
```bash
cargo test multi_agent_collaboration_test
```

### Run Demo
```bash
python3 demo_agent_interaction.py
```

## Agent Communication Protocol

Agents communicate using the enhanced A2A protocol with two new RPC methods:

### agent_query
Query another agent for specific knowledge:
```json
{
  "method": "agent_query",
  "params": {
    "from_agent_id": "orchestrator-agent",
    "to_agent_id": "security-agent",
    "query": "Check this code for vulnerabilities",
    "domain": "security"
  }
}
```

### agent_broadcast
Broadcast capabilities to other agents:
```json
{
  "method": "agent_broadcast",
  "params": {
    "agent_id": "security-agent",
    "broadcast_type": "available",
    "capabilities": ["security_analysis", "vulnerability_detection"]
  }
}
```

## Key Design Principles

1. **Context Efficiency**: Agents ask specific questions rather than sharing entire knowledge bases
2. **No Database Replication**: Knowledge remains with specialists; agents query each other
3. **Configuration Simplicity**: Each agent is just an AGENTS.md file
4. **Zero Configuration**: mDNS enables automatic discovery
5. **Modular Design**: Easy to add new specialized agents

## Example Workflow

User asks: "Review my Python web app for security issues"

1. **Orchestrator** receives request, identifies need for security analysis
2. **Orchestrator** queries **Security Agent**: "Analyze for vulnerabilities"
3. **Security Agent** finds SQL injection risk, queries **Code Review Agent**: "Is this parameterized?"
4. **Code Review Agent** confirms vulnerability
5. **Security Agent** might query **Database Agent** for proper SQL patterns
6. **Orchestrator** aggregates findings and presents report to user

## MCP Server Integration

Agents can use MCP (Model Context Protocol) servers for additional capabilities:

### Built-in MCP Servers
- **arkavo serve**: Provides memory tools (store_memory, search_memory, etc.)
  - Used by orchestrator agent for context preservation
  - Automatically available when running `arkavo serve`

### External MCP Servers (Optional)
Some agent configurations reference external MCP servers that are npm packages:
- **mcp-filesystem**: File system access for agents
- **mcp-git**: Git repository operations

To install external servers (optional):
```bash
npm install -g @modelcontextprotocol/server-filesystem
npm install -g @cyanheads/git-mcp-server
```

**Note**: Agents work without external MCP servers. They will log warnings and continue with reduced capabilities if servers are unavailable.

## Extending the System

To add a new specialized agent:

1. Create directory: `mkdir examples/software-development-lifecycle/new-specialist`
2. Create `AGENTS.md` with appropriate configuration
3. Set unique port number
4. Define purpose that includes keywords for capability detection
5. Start the agent: `cd examples/software-development-lifecycle/new-specialist && arkavo agent run`

The agent will automatically:
- Broadcast its capabilities via mDNS
- Be discoverable by other agents
- Participate in the knowledge sharing network