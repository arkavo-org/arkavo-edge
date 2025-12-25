# Autonomous Refactor Demo

This example demonstrates Arkavo's **Active Context Ledger** and **Multi-Agent Mesh** capabilities.

## The Scenario

A monorepo has 3 microservices depending on a `core_lib`. The `core_lib` introduces a breaking API change (changing a `u32` parameter to `String`), causing compilation errors across the entire workspace.

Running `cargo check` generates **5,000+ lines** of error logs. A standard agent context window would be flooded, leading to "context saturation" or massive cost.

## Arkavo's Approach

### Context Ledger
1. **Noise Detection:** Arkavo intercepts the massive log output.
2. **Ledger Offload:** Instead of feeding all tokens to the LLM, it stores them in the local Context Ledger and injects a pointer: `[ARCHIVED: Build Errors - ID: xyz]`.
3. **Context Rotation:** The agent selectively restores errors for *one service at a time*, fixing them sequentially.

### Multi-Agent Mesh
Four agents collaborate via A2A protocol:
- **refactor-analyzer** - Runs cargo check, categorizes errors, coordinates fixes
- **fixer-alpha** - Fixes service_a
- **fixer-beta** - Fixes service_b
- **fixer-gamma** - Fixes service_c

## Running the Demo

### Option 1: Single Agent (Context Ledger Demo)

```bash
# Setup the broken workspace
./run_demo.sh

# Run Arkavo to fix it
arkavo chat --prompt "Fix the build errors in demo_workspace"
```

### Option 2: Multi-Agent Mesh

```bash
# Launch the mesh (starts 4 agents)
./launch_mesh.sh

# Watch agent logs
tail -f logs/*.log

# Submit a task to the mesh
arkavo task run --prompt "Fix all build errors in demo_workspace"

# Stop the mesh
./stop_mesh.sh
```

## Mesh Architecture

```
                    ┌─────────────────┐
                    │  refactor-      │
                    │  analyzer       │
                    │  (coordinator)  │
                    └────────┬────────┘
                             │ A2A
           ┌─────────────────┼─────────────────┐
           │                 │                 │
           ▼                 ▼                 ▼
    ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
    │ fixer-alpha │   │ fixer-beta  │   │ fixer-gamma │
    │ (service_a) │   │ (service_b) │   │ (service_c) │
    └─────────────┘   └─────────────┘   └─────────────┘
```

## Success Criteria

- Agents do not crash or timeout
- Logs show `[ARCHIVED: ...]` pointers instead of raw error dumps
- Build errors are fixed (cargo check passes after mesh completes)
