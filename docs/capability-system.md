# Arkavo Capability System

The Capability System bridges the gap between **specifications** (what Arkavo can do) and **examples** (how to use it).

## Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    CAPABILITY SYSTEM                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐      CAPABILITIES.md      ┌──────────────┐   │
│  │              │ ◄───────────────────────► │              │   │
│  │    SPECS     │    Unified discovery     │   EXAMPLES   │   │
│  │   (specs/)   │ ◄───────────────────────► │ (examples/)  │   │
│  │              │                           │              │   │
│  │ • 60+ YAML   │   ┌────────────────┐      │ • 25+ demos  │   │
│  │ • 751 BDD    │   │ cargo run --   │      │ • AGENTS.md  │   │
│  │   scenarios  │   │ capabilities   │      │ • tasks.json │   │
│  │ • Invariants │   └───────┬────────┘      │ • README.md  │   │
│  │ • References │           │               │              │   │
│  │              │           │               │              │   │
│  └──────────────┘           │               └──────────────┘   │
│                             │                                    │
│                             ▼                                    │
│                    ┌─────────────────┐                          │
│                    │  Interactive    │                          │
│                    │  Capability     │                          │
│                    │  Browser        │                          │
│                    └─────────────────┘                          │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Components

### CAPABILITIES.md (Root Level)

The central capability registry that maps:
- **Capabilities** → Spec files + Example directories
- **Learning paths** → Progressive skill building
- **Use cases** → Find by what you want to build
- **Complexity ratings** → ⭐ to ⭐⭐⭐⭐⭐

### CLI Capability Browser

```bash
# Interactive mode
cargo xtask capabilities

# List all capabilities
cargo xtask capabilities --list

# Show specific capability
cargo xtask capabilities hrm

# Search capabilities
cargo xtask capabilities --search "encryption"

# Show matrix view
cargo xtask capabilities --matrix
```

The CLI reads `specs/arkavo-edge/index.yaml` directly — each spec IS a capability, no artificial groupings.

### Capability Badges (Example READMEs)

Each example README includes a capability badge linking to specs:

```markdown
<!-- ARKAVO-CAPABILITY: gossip-protocol -->
> **Specs**: [8 scenarios](../../specs/arkavo-edge/gossip-protocol.spec.yaml)
> **Browse**: `cargo xtask capabilities gossip-protocol`
<!-- /ARKAVO-CAPABILITY -->
```

## Usage Workflows

### Workflow: "I want to build X"

1. Check CAPABILITIES.md "By Use Case" section
2. Find relevant capability
3. Run `cargo xtask capabilities <name>`
4. Follow example link

### Workflow: "I found an example, how does it work?"

1. Open example README
2. Click capability badge
3. View BDD specs for expected behavior
4. Cross-reference with implementation

### Workflow: "I'm adding a new feature"

1. Create spec: `specs/arkavo-edge/<feature>.spec.yaml`
2. Create example: `examples/<use-case>/`
3. Add to CAPABILITIES.md matrix
4. Add badge to example README
5. Run `cargo xtask capabilities --matrix` to verify

## Capability Mapping

| Capability | Spec Scenarios | Example | Badge |
|------------|---------------|---------|-------|
| llm-core | 6 | 01-hello-world | ✅ |
| protocol | 19 | mesh | ✅ |
| hrm | 6 | family-travel-mesh | ✅ |
| gossip-protocol | 8 | fleet-immunity | ✅ |
| mcp-tools | 10 | minecraft | ✅ |
| network-security | 17 | secure-agent | ✅ |
| orchestrator | 11 | software-development-lifecycle | ✅ |
| mcp-claude | 9 | code-agent-claude | ✅ |
| gemini | 10 | code-agent-gemini | ✅ |
| autolearn | 8 | self-improvement-swarm | ✅ |

## Benefits

- **Discoverability**: Find relevant examples by capability
- **Traceability**: Specs ↔ Examples bidirectional linking
- **Learning Paths**: Curated progression from beginner to expert
- **Maintenance**: Central registry makes updates easier
- **Onboarding**: New users can self-guide through capabilities
