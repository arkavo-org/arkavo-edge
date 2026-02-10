# Arkavo Capabilities

Interactive capability map for the Arkavo Edge platform.

**Quick Start**: `cargo xtask capabilities`

---

## Capability Matrix

| Capability | Status | Specs | Example | Complexity | Try It |
|------------|--------|-------|---------|------------|--------|
| **Core Agent** | ✅ Stable | [6](specs/arkavo-edge/llm-core.spec.yaml) | [01-hello-world](examples/01-hello-world/) | ⭐ Beginner | `./demo.sh 01-hello-world` |
| **Multi-Agent Mesh** | ✅ Stable | [19](specs/arkavo-edge/protocol.spec.yaml) | [mesh](examples/mesh/) | ⭐⭐ Intermediate | `./mesh.sh start 3` |
| **HRM Orchestration** | ✅ Stable | [6](specs/arkavo-edge/hrm.spec.yaml) | [family-travel-mesh](examples/family-travel-mesh/) | ⭐⭐⭐ Advanced | `./demo.sh family-travel-mesh` |
| **Gossip Learning** | ✅ Stable | [8](specs/arkavo-edge/gossip-protocol.spec.yaml) | [fleet-immunity](examples/fleet-immunity/) | ⭐⭐⭐ Advanced | `./demo.sh fleet-immunity` |
| **MCP Tools** | ✅ Stable | [10](specs/arkavo-edge/mcp-tools.spec.yaml) | [minecraft](examples/minecraft/) | ⭐⭐ Intermediate | See [RUNBOOK](examples/minecraft/RUNBOOK.md) |
| **TDF Encryption** | ✅ Stable | [9](specs/arkavo-edge/tdf.spec.yaml) | — | ⭐⭐ Intermediate | `cargo test -p arkavo-tdf` |
| **Preflight Policies** | ✅ Stable | [17](specs/arkavo-edge/router.spec.yaml) | [secure-agent](examples/secure-agent/) | ⭐⭐ Intermediate | `./demo.sh secure-agent` |
| **SDLC Team** | ✅ Stable | [11](specs/arkavo-edge/orchestrator.spec.yaml) | [software-development-lifecycle](examples/software-development-lifecycle/) | ⭐⭐⭐⭐ Expert | `./launch.sh` in dir |
| **Auto-Learning** | 🧪 Beta | [8](specs/arkavo-edge/autolearn.spec.yaml) | [self-improvement-swarm](examples/self-improvement-swarm/) | ⭐⭐⭐⭐ Expert | See [RUNBOOK](examples/self-improvement-swarm/RUNBOOK.md) |
| **TØRG Constraints** | 🧪 Beta | [8](specs/arkavo-edge/torg.spec.yaml) | — | ⭐⭐⭐⭐⭐ Research | `cargo test -p arkavo-torg` |

**Legend**: ⭐ Beginner | ⭐⭐ Intermediate | ⭐⭐⭐ Advanced | ⭐⭐⭐⭐ Expert | ⭐⭐⭐⭐⭐ Research

---

## Learning Pathways

### 🚀 First 5 Minutes
Just want to see it work?
```bash
./examples/demo.sh 01-hello-world
```
**You'll learn**: Basic agent configuration, local LLM inference

### 🌱 Foundation (30 min)
Understand core concepts:
1. [01-hello-world](examples/01-hello-world/) - Single agent basics
2. [code-agent-claude](examples/code-agent-claude/) - Cloud LLM integration
3. [secure-agent](examples/secure-agent/) - Input validation

### 🌿 Multi-Agent Basics (1 hour)
Agents working together:
1. [software-development-simple](examples/software-development-simple/) - 3-agent collaboration
2. [orchestrator-agent](examples/orchestrator-agent/) - Task routing
3. [mesh](examples/mesh/) - Dynamic peer discovery

### 🌳 Advanced Patterns (2 hours)
Production-grade orchestration:
1. [family-travel-mesh](examples/family-travel-mesh/) - HRM with Thompson Sampling
2. [fleet-immunity](examples/fleet-immunity/) - Gossip learning
3. [hyperforum-council](examples/hyperforum-council/) - Discourse management

### 🏭 Production Systems (2+ hours)
Full-stack implementations:
1. [software-development-lifecycle](examples/software-development-lifecycle/) - 12-agent SDLC
2. [minecraft](examples/minecraft/) - MCP tools integration
3. [autonomous_refactor](examples/autonomous_refactor/) - Self-improving code

---

## By Use Case

### I want to...

**...build a coding assistant**
- Start: [code-agent-claude](examples/code-agent-claude/)
- Scale: [software-development-lifecycle](examples/software-development-lifecycle/)
- Specs: [github](specs/arkavo-edge/github.spec.yaml), [git](specs/arkavo-edge/git.spec.yaml)

**...create a multi-agent team**
- Start: [software-development-simple](examples/software-development-simple/)
- Scale: [software-development-lifecycle](examples/software-development-lifecycle/)
- Specs: [protocol](specs/arkavo-edge/protocol.spec.yaml), [orchestrator](specs/arkavo-edge/orchestrator.spec.yaml)

**...add learning to my agents**
- Example: [fleet-immunity](examples/fleet-immunity/)
- Specs: [gossip-protocol](specs/arkavo-edge/gossip-protocol.spec.yaml), [autolearn](specs/arkavo-edge/autolearn.spec.yaml)

**...use external tools**
- Example: [minecraft](examples/minecraft/)
- Specs: [mcp-tools](specs/arkavo-edge/mcp-tools.spec.yaml), [mcp-runtime](specs/arkavo-edge/mcp-runtime.spec.yaml)

**...secure my agents**
- Example: [secure-agent](examples/secure-agent/)
- Specs: [network-security](specs/arkavo-edge/network-security.spec.yaml), [authorization](specs/arkavo-edge/authorization.spec.yaml)

---

## By Component

### Communication & Discovery
| Component | Spec | Example | Description |
|-----------|------|---------|-------------|
| A2A Protocol | [19 scenarios](specs/arkavo-edge/protocol.spec.yaml) | [mesh](examples/mesh/) | Agent-to-agent communication |
| mDNS Discovery | [6 scenarios](specs/arkavo-edge/device-identity.spec.yaml) | All examples | Zero-config peer discovery |
| Gossip Protocol | [8 scenarios](specs/arkavo-edge/gossip-protocol.spec.yaml) | [fleet-immunity](examples/fleet-immunity/) | P2P knowledge sharing |

### Orchestration
| Component | Spec | Example | Description |
|-----------|------|---------|-------------|
| HRM | [6 scenarios](specs/arkavo-edge/hrm.spec.yaml) | [family-travel-mesh](examples/family-travel-mesh/) | Hierarchical task orchestration |
| Task Orchestration | [8 scenarios](specs/arkavo-edge/task-orchestration.spec.yaml) | [orchestrator-agent](examples/orchestrator-agent/) | Task planning & execution |
| Router | [17 scenarios](specs/arkavo-edge/router.spec.yaml) | All multi-agent | LLM routing & quality gates |

### Security & Privacy
| Component | Spec | Example | Description |
|-----------|------|---------|-------------|
| TDF | [9 scenarios](specs/arkavo-edge/tdf.spec.yaml) | — | Trusted Data Format encryption |
| Registration | [12 scenarios](specs/arkavo-edge/registration.spec.yaml) | — | Device onboarding |
| Network Security | [17 scenarios](specs/arkavo-edge/network-security.spec.yaml) | [secure-agent](examples/secure-agent/) | Secure defaults |

### Intelligence
| Component | Spec | Example | Description |
|-----------|------|---------|-------------|
| AutoLearn | [8 scenarios](specs/arkavo-edge/autolearn.spec.yaml) | [self-improvement-swarm](examples/self-improvement-swarm/) | Self-improving agents |
| Critic | [17 scenarios](specs/arkavo-edge/critic.spec.yaml) | [software-development-lifecycle](examples/software-development-lifecycle/) | Response verification |
| Context | [15 scenarios](specs/arkavo-edge/context.spec.yaml) | [rlm-large-context](examples/rlm-large-context/) | Context management |

---

## Quick Reference

### Start an Example
```bash
# Interactive mode
./examples/demo.sh

# Specific example
./examples/demo.sh fleet-immunity

# With specific model
./examples/demo.sh --model glm software-development-simple
```

### Run Spec Tests
```bash
# All tests
cargo test

# Specific component
cargo test -p arkavo-protocol
cargo test -p arkavo-hrm
```

### Validate Specs
```bash
# Requires ajv-cli: npm install -g ajv-cli
npx ajv-cli validate -s specs/schema.json -d "specs/**/*.spec.yaml"
```

---

## Contributing

When adding a new capability:

1. **Add spec**: `specs/arkavo-edge/<component>.spec.yaml`
2. **Add example**: `examples/<use-case-name>/`
3. **Update this file**: Add to matrix and pathways
4. **Link them**: Example README should reference spec scenarios

See [specs/README.md](specs/README.md) for spec format.
See [examples/README.md](examples/README.md) for example structure.
