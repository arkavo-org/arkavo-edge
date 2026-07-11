# Hello World Agent

<!-- ARKAVO-CAPABILITY: llm-core -->
> **Specs**: [6 scenarios](../../specs/arkavo-edge/llm-core.spec.yaml)
> **Browse**: `cargo xtask capabilities llm-core`
<!-- /ARKAVO-CAPABILITY -->

Your first Arkavo agent in 5 minutes.

## What You'll Learn

- How to configure an agent with a SwarmKit kit
- How to start and interact with an agent
- The basic agent lifecycle

## Prerequisites

```bash
# Build Arkavo (from repo root)
cargo build
```

## Quick Start

```bash
# From this directory
./run.sh
```

That's it! The agent will start and respond to your prompt.

## What's Happening

1. `run.sh` starts the agent using `hello-agent.swarmkit.yaml`
2. The agent loads the `ministral-3b` model (downloads on first run)
3. The agent processes the task from `tasks.json`
4. You see the response

## Files

| File | Purpose |
|------|---------|
| `hello-agent.swarmkit.yaml` | Agent configuration (name, purpose, model) |
| `tasks.json` | Demo tasks to run |
| `run.sh` | Launch script |

## Next Steps

- Try `02-single-agent/` to explore different LLM backends
- Try `03-multi-agent-basics/` to see agents collaborate
- Read `../CONCEPTS.md` for deeper understanding
