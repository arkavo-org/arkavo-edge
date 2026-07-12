# HYPERforum AI Council

Multi-agent orchestration for AI-augmented discourse in HYPERforum.

## Architecture

```
                    HYPERforum AI Council

┌─────────────────────────────────────────────────────────────────┐
│                        CONDUCTOR                                 │
│                       (Port 8501)                                │
│   Strategic task decomposition for discourse questions           │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      v
┌─────────────────────────────────────────────────────────────────┐
│                         ROUTER                                   │
│                       (Port 8502)                                │
│   Thompson Sampling agent selection based on discourse category  │
└─────────────────────┬───────────────────────────────────────────┘
                      │
    ┌─────────┬───────┼───────┬─────────┬─────────┐
    v         v       v       v         v         v
┌───────┐ ┌───────┐ ┌───────┐ ┌───────┐ ┌───────┐
│Critical│ │Resear-│ │Synthe-│ │Devil's│ │Facili-│
│Analyst │ │cher   │ │sizer  │ │Advoca-│ │tator  │
│(8510)  │ │(8511) │ │(8512) │ │te     │ │(8514) │
│        │ │       │ │       │ │(8513) │ │       │
└────────┘ └───────┘ └───────┘ └───────┘ └───────┘
                      │
                      v
┌─────────────────────────────────────────────────────────────────┐
│                         CRITIC                                   │
│                       (Port 8503)                                │
│   Discourse policy verification + semantic coherence checks      │
└─────────────────────┬───────────────────────────────────────────┘
                      │
                      v
┌─────────────────────────────────────────────────────────────────┐
│                       SYNTHESIS                                  │
│                       (Port 8504)                                │
│   Final response synthesis from all specialist contributions     │
└─────────────────────────────────────────────────────────────────┘
```

## Quick Start

```bash
# Launch all agents (reads roles/ports from hyperforum-council.swarmkit.yaml)
./launch.sh

# Submit a deliberation task to the Conductor
curl -X POST http://localhost:8501/tasks -d @tasks.json

# Stop all agents
./stop.sh
```

## Specialist Roles

| Agent | Purpose | Skills |
|-------|---------|--------|
| **Critical Analyst** | Logical analysis, fallacy detection | argument_mapping, bias_detection |
| **Researcher** | Fact-finding, evidence gathering | source_verification, context_gathering |
| **Synthesizer** | Integration, pattern recognition | perspective_integration, narrative_construction |
| **Devil's Advocate** | Constructive criticism, stress-testing | counterargument_generation, steelmanning |
| **Facilitator** | Discussion management, conflict resolution | process_optimization, participation_balancing |

## Configuration

- `hyperforum-council.swarmkit.yaml` - the 9-role SwarmKit mesh definition
- `config/hrm_defaults.yaml` - HRM orchestration settings (router scoring weights,
  guardrails, memory tiers, deliberation rounds). Its `encryption.kas_url` block
  (OpenTDF for sensitive insights) has no kit-level home yet:
  `runtime.kas` (see `crates/arkavo-swarmkit/src/runtime_config.rs`) is
  DID/trusted-root shaped, not a KAS URL — not yet modeled in SwarmKit.
- `agents/critic/policies/discourse_quality.yaml` - Discourse quality policy,
  referenced in the Critic role's skill instructions in the kit

## Integration with HYPERforum

HYPERforum connects via A2A protocol (JSON-RPC 2.0 over WebSocket):

1. mDNS discovery finds `_a2a._tcp` services
2. Connect to arkavo-edge instance
3. Call `hrm.orchestrate` with council request
4. Stream receives `HRMDelta` notifications
5. Final synthesis returned with optional OpenTDF encryption
