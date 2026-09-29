# Arkavo Examples

Learn to build AI agent systems through hands-on examples.

> 🎯 **New to Arkavo?** Try the [Capability Browser](../docs/CAPABILITIES.md) to find what you need. The browser commands run from a source checkout:
> `cargo xtask capabilities` - Interactive browser
> `cargo xtask capabilities --matrix` - Quick overview

## Shipped SwarmKits

Four SwarmKits ship as runnable examples. Each is a single YAML manifest with inline-signed skills that exercises the SwarmKit runtime end-to-end (parse + validate + skill resolution + per-role ARP construction).

| Kit | Domain | Roles | Validate (from the repository root) |
|---|---|---|---|
| [campaign-kit](campaign-kit/README.md) | Marketing | analyst → copy → critic | `arkavo kit validate examples/campaign-kit/campaign-kit.swarmkit.yaml` |
| [code-review-kit](code-review-kit/README.md) | Developer | reviewer → security_auditor → test_writer | `arkavo kit validate examples/code-review-kit/code-review-kit.swarmkit.yaml` |
| [vrm-production-kit](vrm-production-kit/README.md) | Creative | prompt_designer → vrm_assembler → validator | `arkavo kit validate examples/vrm-production-kit/vrm-production-kit.swarmkit.yaml` |
| [compliance-kit](compliance-kit/README.md) | Regulated | pii_classifier → policy_enforcer → auditor | `arkavo kit validate examples/compliance-kit/compliance-kit.swarmkit.yaml` |

`arkavo kit validate` accepts several paths, so one command can check all four. From a source checkout without an installed binary, the equivalent is `cargo run -p arkavo-swarmkit --example validate_kit -- <path>`.

Each kit's README shows how to load the kit into the gateway and how to start its roles. Loading a kit builds per-role policy, trace, and panel entries; it does not start the roles or run the pipeline between them. See [What a kit launch does today](../docs/SWARMKIT.md#what-a-kit-launch-does-today).

Each kit ships with its own `README.md` in `examples/<kit-name>/` plus an integration test in `crates/arkavo-swarmkit-runtime/tests/<kit-name>_skill_resolver.rs` that asserts every role's skills resolve with `verified=true`.

## Quick Start (5 minutes)

```bash
# 1. Build Arkavo (from repo root)
cargo build

# 2. Browse capabilities interactively
cargo xtask capabilities

# 3. Or run your first agent directly
cd examples/01-hello-world
./run.sh
```

## Capability Map

Each example demonstrates specific Arkavo capabilities. Find by use case:

| I want to... | Start Here | Key Capabilities |
|--------------|------------|------------------|
| Build a coding assistant | [code-agent-claude](code-agent-claude/) | Core Agent, MCP Tools |
| Create a multi-agent team | [software-development-simple](software-development-simple/) | Multi-Agent Mesh, A2A Protocol |
| Add learning to agents | [learning-mesh](learning-mesh/) | Lesson-Informed Prompting, Thompson Sampling |
| Use external tools | [minecraft](minecraft/) | MCP Tools |
| Secure my agents | [secure-agent](secure-agent/) | Preflight Policies |
| Bridge to external AI | [openclaw-a2a-bridge](openclaw-a2a-bridge/) | A2A Protocol, TDF, Preflight, Budget |
| Build production system | [software-development-lifecycle](software-development-lifecycle/) | HRM, Orchestrator |

See [CAPABILITIES.md](../docs/CAPABILITIES.md) for the full capability matrix.

## Learning Path

Progress from simple to complex, building skills incrementally. The levels are a suggested order, not directories: `examples/` is flat, and every example is a directory directly under it.

| Level | Examples | What You'll Learn | Time |
|-------|----------|-------------------|------|
| **Hello world** | [01-hello-world](01-hello-world/) | Agent basics, SwarmKit config | 5 min |
| **Single agent** | [code-agent-claude](code-agent-claude/), [code-agent-gemini](code-agent-gemini/), [secure-agent](secure-agent/) | LLM backends, API keys, policies | 30 min |
| **Multi-agent basics** | [software-development-simple](software-development-simple/), [orchestrator-agent](orchestrator-agent/) | Agent collaboration, A2A protocol | 1 hr |
| **Advanced patterns** | [family-travel-mesh](family-travel-mesh/), [fleet-immunity](fleet-immunity/), [hyperforum-council](hyperforum-council/) | Orchestration, learning, discourse | 2 hr |
| **Production** | [software-development-lifecycle](software-development-lifecycle/), [minecraft](minecraft/) | Full systems, MCP tools | 2+ hr |
| **Specialized** | [rlm-large-context](rlm-large-context/) | Large context handling | 1 hr |

## All Examples

### Hello world
Your first agent. Start here.

| Example | Description | Requirements |
|---------|-------------|--------------|
| [01-hello-world](01-hello-world/) | One-shot chat that answers a greeting | None (uses a local model) |

### Single agent
Individual agent patterns with different backends.

| Example | Description | Requirements |
|---------|-------------|--------------|
| [code-agent-claude](code-agent-claude/) | Coding with Claude | `ANTHROPIC_API_KEY` |
| [code-agent-gemini](code-agent-gemini/) | Coding with Gemini | `GEMINI_API_KEY` |
| [secure-agent](secure-agent/) | Preflight policy enforcement | None |

### Multi-agent basics
Simple multi-agent collaboration.

| Example | Agents | Description |
|---------|--------|-------------|
| [software-development-simple](software-development-simple/) | 3 | Project manager, coder, tester |
| [orchestrator-agent](orchestrator-agent/) | 1+ | Central task routing |

### Advanced patterns
Advanced orchestration and learning patterns.

| Example | Pattern | Description |
|---------|---------|-------------|
| [family-travel-mesh](family-travel-mesh/) | HRM | Hierarchical orchestration with Thompson Sampling |
| [fleet-immunity](fleet-immunity/) | Gossip | Peer-to-peer learning between rovers |
| [learning-mesh](learning-mesh/) | Learning | Quality-aware routing with lesson-informed prompting |
| [hyperforum-council](hyperforum-council/) | Discourse | AI-powered discussion management |
| [autonomous_refactor](autonomous_refactor/) | Ledger | Context tracking for code refactoring |
| [evofabric](evofabric/) | AST Ops | Typed code evolution with verification |
| [openclaw-a2a-bridge](openclaw-a2a-bridge/) | Bridge | A2A protocol bridge with security comparison |

### Production
Production-ready multi-agent systems.

| Example | Agents | Description |
|---------|--------|-------------|
| [software-development-lifecycle](software-development-lifecycle/) | 12 | Full SDLC with domain specialists |
| [minecraft](minecraft/) | 5 | Game bot with MCP tools |

### Specialized
Special capabilities.

| Example | Description |
|---------|-------------|
| [rlm-large-context](rlm-large-context/) | Handle 100K+ token contexts |

## Core Concepts

Read [CONCEPTS.md](CONCEPTS.md) to understand:

- **Agent Architecture** - What agents are and how they work
- **SwarmKit Configuration** - Kit manifest format (`*.swarmkit.yaml`)
- **mDNS Discovery** - Zero-config agent discovery
- **A2A Protocol** - Agent-to-agent communication
- **HRM Pattern** - Hierarchical orchestration
- **Thompson Sampling** - Intelligent agent selection
- **Gossip Learning** - Peer-to-peer knowledge sharing
- **MCP Integration** - External tool usage
- **Preflight Policies** - Input validation and safety

## Prerequisites

### Required

```bash
# Rust toolchain
rustup --version

# Build the project
cargo build
```

### For Cloud LLMs

```bash
# Claude examples
export ANTHROPIC_API_KEY="your-key"

# Gemini examples
export GEMINI_API_KEY="your-key"
```

### Verify Setup

```bash
# Check binary exists
ls target/debug/arkavo

# Check mDNS works (macOS)
dns-sd -B _a2a._tcp local.

# Check mDNS works (Linux)
avahi-browse -art | grep a2a
```

## Demo Mode

Run an interactive showcase of all capabilities:

```bash
./demo.sh
```

Or run a specific scenario:

```bash
./transition.sh software-development-simple
```

## Management Scripts

| Script | Purpose |
|--------|---------|
| `demo.sh` | Run all demo scenarios sequentially |
| `mesh.sh` | Manage a generic agent mesh (start/stop/status) |
| `transition.sh` | Run tasks from a single scenario |

### mesh.sh Usage

```bash
# Start 8 agents
./mesh.sh start 8

# Check status
./mesh.sh status

# Stop all agents
./mesh.sh stop
```

## Port Conventions

| Range | Purpose | Examples |
|-------|---------|----------|
| 8340-8341 | Orchestrators | orchestrator-agent |
| 8342-8353 | SDLC specialists | software-development-* |
| 8401-8412 | HRM mesh | family-travel-mesh |
| Dynamic | Fleet/mesh | fleet-immunity, mesh.sh |
| 8360-8361 | A2A Bridge | openclaw-a2a-bridge |

Use `lsof -i :PORT` to check if a port is in use.

## Standard Example Structure

Each example follows this structure:

```
example-name/
├── README.md               # Overview and quick start
├── RUNBOOK.md              # Step-by-step guide
├── example-name.swarmkit.yaml  # SwarmKit manifest (single-role or multi-role kit)
├── tasks.json              # Demo tasks
├── launch.sh                # Start the example
└── stop.sh                  # Stop the example
```

## Creating New Examples

1. Copy `01-hello-world` as a template
2. Write `<name>.swarmkit.yaml` — start from `arkavo kit init <name>` (single role) or hand-author a multi-role kit modeled on `campaign-kit/campaign-kit.swarmkit.yaml`; validate with `arkavo kit validate <path>`
3. Add tasks to `tasks.json`
4. Write `README.md` and `RUNBOOK.md`
5. Create `launch.sh` and `stop.sh`
6. Add to this README

Converting a legacy AGENTS.md into a starting-point kit: `arkavo kit migrate-from-agents-md --in <file> --out <kit>` (best-effort — hand-finish preflight/KAS/budget fields it can't map).

## Troubleshooting

### Agent Won't Start

```bash
# Kill orphan processes
pkill -f "arkavo agent"

# Check for port conflicts
lsof -i :8342
```

### Model Download Fails

```bash
# Check connectivity
curl -I https://huggingface.co

# Enable debug logging
RUST_LOG=debug ./launch.sh
```

### mDNS Discovery Fails

```bash
# macOS: Check Bonjour
dns-sd -B _a2a._tcp local.

# Linux: Check Avahi
systemctl status avahi-daemon
```

### API Key Errors

```bash
# Verify key is set
echo $ANTHROPIC_API_KEY
echo $GEMINI_API_KEY

# Set for current session
export ANTHROPIC_API_KEY="sk-ant-..."
```

## Contributing

When adding or modifying examples:

1. Test the RUNBOOK.md manually from a fresh state
2. Ensure mDNS discovery works
3. Check for port conflicts with other examples
4. Update this README with your example
5. Add to the appropriate learning level
