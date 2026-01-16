# Arkavo Examples

Learn to build AI agent systems through hands-on examples.

## Quick Start (5 minutes)

```bash
# 1. Build Arkavo (from repo root)
cargo build

# 2. Run your first agent
cd examples/01-hello-world
./run.sh
```

## Learning Path

Progress from simple to complex, building skills incrementally.

| Level | Examples | What You'll Learn | Time |
|-------|----------|-------------------|------|
| **01-hello-world** | Minimal agent | Agent basics, AGENTS.md config | 5 min |
| **02-single-agent** | Claude, Gemini, secure | LLM backends, API keys, policies | 30 min |
| **03-multi-agent-basics** | Dev team, orchestrator | Agent collaboration, A2A protocol | 1 hr |
| **04-advanced-patterns** | HRM, fleet, hyperforum | Orchestration, learning, discourse | 2 hr |
| **05-production** | SDLC, minecraft | Full systems, MCP tools | 2+ hr |
| **06-specialized** | RLM | Large context handling | 1 hr |

## All Examples

### 01-hello-world
Your first agent. Start here.
- Single agent responding to a greeting
- No API keys needed (uses local model)

### 02-single-agent
Individual agent patterns with different backends.

| Example | Description | Requirements |
|---------|-------------|--------------|
| `code-agent-claude` | Coding with Claude | `ANTHROPIC_API_KEY` |
| `code-agent-gemini` | Coding with Gemini | `GEMINI_API_KEY` |
| `secure-agent` | Preflight policy enforcement | None |

### 03-multi-agent-basics
Simple multi-agent collaboration.

| Example | Agents | Description |
|---------|--------|-------------|
| `software-development-simple` | 3 | Project manager, coder, tester |
| `orchestrator-agent` | 1+ | Central task routing |

### 04-advanced-patterns
Advanced orchestration and learning patterns.

| Example | Pattern | Description |
|---------|---------|-------------|
| `family-travel-mesh` | HRM | Hierarchical orchestration with Thompson Sampling |
| `fleet-immunity` | Gossip | Peer-to-peer learning between rovers |
| `hyperforum-council` | Discourse | AI-powered discussion management |
| `autonomous_refactor` | Ledger | Context tracking for code refactoring |

### 05-production
Production-ready multi-agent systems.

| Example | Agents | Description |
|---------|--------|-------------|
| `software-development-lifecycle` | 12 | Full SDLC with domain specialists |
| `minecraft` | 5 | Game bot with MCP tools |

### 06-specialized
Special capabilities.

| Example | Description |
|---------|-------------|
| `rlm-large-context` | Handle 100K+ token contexts |

## Core Concepts

Read [CONCEPTS.md](CONCEPTS.md) to understand:

- **Agent Architecture** - What agents are and how they work
- **AGENTS.md** - Configuration format
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

Use `lsof -i :PORT` to check if a port is in use.

## Standard Example Structure

Each example follows this structure:

```
example-name/
├── README.md        # Overview and quick start
├── RUNBOOK.md       # Step-by-step guide
├── AGENTS.md        # Agent config (single) or agents/ (multi)
├── tasks.json       # Demo tasks
├── launch.sh        # Start the example
└── stop.sh          # Stop the example
```

## Creating New Examples

1. Copy `01-hello-world` as a template
2. Update `AGENTS.md` with your agent config
3. Add tasks to `tasks.json`
4. Write `README.md` and `RUNBOOK.md`
5. Create `launch.sh` and `stop.sh`
6. Add to this README

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
