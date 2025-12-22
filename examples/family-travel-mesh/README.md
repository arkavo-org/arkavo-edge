# Family Travel Mesh Demo - HRM Orchestration

This demo implements the complete HRM-Style Orchestration architecture from Issue #236 using a "Family Travel Planning" use case.

## Architecture

```
┌─────────────────────┐
│     CONDUCTOR       │ ← Slow Loop: Task decomposition
│    (Port 8401)      │
└──────────┬──────────┘
           │
           v
┌─────────────────────┐
│      ROUTER         │ ← Medium Loop: Thompson Sampling
│    (Port 8402)      │
└──────────┬──────────┘
           │
    ┌──────┼──────┐
    v      v      v
┌───────┐ ┌───────┐ ┌───────┐
│Vegas  │ │Family │ │Budget │  ← Fast Loop: Specialists
│Guide  │ │Activ. │ │Optim. │
│(8410) │ │(8411) │ │(8412) │
└───────┘ └───────┘ └───────┘
           │
           v
┌─────────────────────┐
│      CRITIC         │ ← Verification Pipeline
│    (Port 8403)      │
└─────────────────────┘
           │
           v
┌─────────────────────┐
│   MEMORY SERVICE    │ ← Context Management
│    (Port 8404)      │
└─────────────────────┘
```

## Quick Start

1. Build Arkavo CLI:
```bash
cargo build
```

2. Start the mesh:
```bash
cd examples/family-travel-mesh
./launch_mesh.sh
```

3. Run the demo:
```bash
./run_task.sh
```

4. Stop the mesh:
```bash
./stop_mesh.sh
```

## Demo Scenario

The demo plans a Friday afternoon in Las Vegas for a family with twin toddlers:
- **Travelers**: 2 adults, 2 children (age 3)
- **Time Window**: 12:00-18:00
- **Budget**: $200

The HRM system:
1. **Conductor** decomposes into subtasks
2. **Router** selects specialists via Thompson Sampling
3. **Specialists** execute with burst contracts
4. **Critic** verifies outputs against family safety policy
5. **Memory Service** manages context across bursts

## Adversarial Testing

Test fault tolerance with hazard injection:

```bash
# ADV-01: Drunk Agent (schema corruption)
./inject_hazard.sh drunk

# ADV-02: Lazy Agent (minimal compliance)
./inject_hazard.sh lazy

# ADV-03: Infinite Loop Detection
./inject_hazard.sh loop
```

## Agent Configurations

| Agent | Port | Purpose |
|-------|------|---------|
| Conductor | 8401 | Task decomposition and orchestration |
| Router | 8402 | Thompson Sampling agent selection |
| Vegas Guide | 8410 | Local knowledge specialist |
| Family Activities | 8411 | Child-friendly expert |
| Budget Optimizer | 8412 | Cost optimization |
| Critic | 8403 | Verification pipeline |
| Memory | 8404 | Context management |

## Policy Enforcement

The Critic enforces `family_safety.yaml`:
- **Prohibited**: gambling, casinos, adult entertainment, bars
- **Required**: age-appropriate, stroller-accessible venues
- **Age Restrictions**: No 21+ or 18+ venues

## Expected Output

```
[CONDUCTOR] Task created: task_001
[CONDUCTOR] Decomposing into subtasks...
[CONDUCTOR] > Subtask 1: Find age-appropriate activities
[CONDUCTOR] > Subtask 2: Verify safety and accessibility
[CONDUCTOR] > Subtask 3: Optimize for budget

[ROUTER] Thompson Sampling scores:
[ROUTER]   | vegas-guide:       0.68 (Beta(35,12))
[ROUTER]   | family-activities: 0.94 (Beta(112,11)) <- SELECTED
[ROUTER]   | budget-optimizer:  0.51 (Beta(78,11))

[SPECIALIST:family-activities] Executing burst contract...

[CRITIC] Verification pipeline:
[CRITIC]   [0] SchemaCheck: OK (0.8ms)
[CRITIC]   [1] LintCheck: OK (1.2ms)
[CRITIC]   [2] PolicyCheck: OK (0.6ms)
[CRITIC]   [3] SemanticCheck: OK (45ms)
[CRITIC] APPROVED

[CONDUCTOR] Task completed. Total cost: $0.31
```

## Troubleshooting

### Agents not starting
- Check if ports 8401-8404 and 8410-8412 are available
- Verify binary exists: `ls ../../target/debug/arkavo`

### Discovery failures
- Ensure mDNS is enabled in AGENTS.md files
- Wait for discovery (5 seconds after launch)

### Policy violations
- Check `agents/critic/policies/family_safety.yaml`
- Review agent outputs for prohibited venue types

## Project Structure

```
family-travel-mesh/
├── README.md              # This file
├── launch_mesh.sh         # Start all agents
├── stop_mesh.sh           # Stop all agents
├── run_task.sh            # Execute demo
├── inject_hazard.sh       # Adversarial testing
├── config/
│   └── hrm_defaults.yaml  # HRM configuration
├── agents/
│   ├── conductor/         # Task orchestrator
│   ├── router/            # Agent selector
│   ├── specialists/       # Domain experts
│   ├── critic/            # Verification
│   └── memory/            # Context service
├── scenarios/
│   ├── vegas_friday.json  # Main scenario
│   └── adversarial/       # Fault injection
└── logs/                  # Agent logs
```

## Learn More

- [HRM Implementation (PR #423)](https://github.com/arkavo-org/arkavo-edge/pull/423)
- [Issue #236: HRM-Style Orchestration](https://github.com/arkavo-org/arkavo-edge/issues/236)
- [arkavo-hrm crate](../../crates/arkavo-hrm/)
- [arkavo-critic crate](../../crates/arkavo-critic/)
