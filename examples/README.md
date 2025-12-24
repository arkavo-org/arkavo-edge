# Arkavo Examples

This directory contains example configurations demonstrating Arkavo's agent capabilities.

## Available Examples

| Example | Description | Complexity |
|---------|-------------|------------|
| [claude-code-agent](claude-code-agent/) | AI coding assistant with Claude | Single agent |
| [gemini-code-agent](gemini-code-agent/) | AI coding assistant with Gemini | Single agent |
| [orchestrator-agent](orchestrator-agent/) | Central task router for agent mesh | Single agent |
| [fleet-immunity](fleet-immunity/) | Multi-rover learning through adversity | Multi-agent mesh |
| [family-travel-mesh](family-travel-mesh/) | HRM-style hierarchical orchestration | Multi-agent mesh |
| [software-development-lifecycle](software-development-lifecycle/) | 12-agent SDLC collaboration | Multi-agent mesh |
| [software-development-simple](software-development-simple/) | Simplified 3-agent development team | Multi-agent mesh |

## Example Categories

### Single Agent Examples
Demonstrate individual agent capabilities with specific LLM backends:
- `claude-code-agent` - Uses Anthropic Claude API
- `gemini-code-agent` - Uses Google Gemini API
- `orchestrator-agent` - Central coordinator (works with any LLM)

### Multi-Agent Mesh Examples
Demonstrate agent-to-agent collaboration:
- `fleet-immunity` - Learning propagation between rovers
- `family-travel-mesh` - Hierarchical Reasoning Model (HRM) pattern
- `software-development-lifecycle` - Domain specialist collaboration
- `software-development-simple` - Minimal multi-agent setup

## Standard Example Structure

Each example should include:

```
example-name/
├── README.md              # Overview, architecture, quick start
├── RUNBOOK.md             # Step-by-step execution guide
├── AGENTS.md              # Agent configuration (single agent)
├── agents/                # Agent configurations (multi-agent)
│   └── agent-name/
│       └── AGENTS.md
├── launch_*.sh            # Startup script with prereq checks
├── stop_*.sh              # Graceful shutdown script
├── logs/                  # Runtime logs (gitignored)
└── test_example.sh        # Automated validation (optional)
```

## RUNBOOK.md Format

Every example includes a RUNBOOK.md with:

1. **What This Example Demonstrates** - Key capabilities shown
2. **Prerequisites** - Build commands, port checks
3. **Step-by-Step Execution** - Commands with "what to watch for"
4. **Automated Validation** - Script for CI/CD testing
5. **Common Failure Modes** - Troubleshooting guide
6. **Architecture Notes** - Design decisions

## Running Examples

### Quick Start

```bash
# Build Arkavo
cargo build

# Run any example
cd examples/<example-name>
./launch_*.sh

# Follow the RUNBOOK.md for detailed steps
```

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `ANTHROPIC_API_KEY` | Required for claude-code-agent |
| `GEMINI_API_KEY` | Required for gemini-code-agent |
| `RUST_LOG` | Debug logging (e.g., `RUST_LOG=debug`) |

## Agent Discovery

All multi-agent examples use mDNS for zero-configuration discovery:
- Service type: `_a2a._tcp.local.`
- Agents advertise their capabilities automatically
- No hardcoded peer addresses required

Check discovery status:
```bash
dns-sd -B _a2a._tcp local.
```

## Port Conventions

| Port Range | Purpose |
|------------|---------|
| 8340-8341 | Orchestrator agents |
| 8342-8353 | SDLC domain specialists |
| 8401-8412 | HRM mesh (conductor, router, specialists) |
| Dynamic | Fleet immunity rovers (mDNS-assigned) |

## Creating New Examples

1. Copy an existing example as a template
2. Update AGENTS.md with your agent's purpose and model
3. Create launch/stop scripts using the standard pattern
4. Write a RUNBOOK.md with "what to watch for" guidance
5. Add to this README.md

### Script Template

```bash
#!/bin/bash
# launch_example.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"

# Check prerequisites
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Build arkavo first: cargo build"
    exit 1
fi

# Start agent
cd "$SCRIPT_DIR"
"$BINARY" agent run
```

## Contributing

When adding or modifying examples:
- Test the full RUNBOOK.md manually
- Verify mDNS discovery works
- Check port conflicts with other examples
- Update this README.md if adding new examples
