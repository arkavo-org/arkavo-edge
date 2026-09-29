# Identity-only mesh fixtures

Three minimal roles (`agent-0`, `agent-1`, `agent-2`) in a single kit,
`identity-only.swarmkit.yaml`, that declare nothing but identity, listen
address, and capability hints. The orchestrator's kit-apply flow uses
these as the pool of agents that get hyperspecialized into roles when a
SwarmKit is applied.

There is no purpose, no model selection, no MCP tool list, and no API
tokens until the orchestrator ships a TDF-wrapped
`AgentSpecializationBundle` to the agent over A2A.

## Demo

```bash
# 1. Start the three identity-only agents (each in its own terminal).
arkavo agent -c examples/identity-only/identity-only.swarmkit.yaml -n agent-0 -p 8341
arkavo agent -c examples/identity-only/identity-only.swarmkit.yaml -n agent-1 -p 8342
arkavo agent -c examples/identity-only/identity-only.swarmkit.yaml -n agent-2 -p 8343

# 2. Start the orchestrator agent.
arkavo agent -c examples/orchestrator-agent/orchestrator-agent.swarmkit.yaml -n orchestrator -p 8340
```

From a source checkout without an installed binary, replace `arkavo` with `cargo run -p arkavo --`.

## Applying a kit

There is no `arkavo orchestrator apply-kit` command. The apply pipeline
is the `apply_swarmkit` agent tool
(`crates/arkavo-server/src/server/swarm_apply_tool.rs`), which the
orchestrator agent calls while handling a task. It takes
`manifest_path` and `org` (both required) and an optional `repo`, and is
registered only in builds with the `kas` and `iroh` features, on an
agent that has an Iroh node.

When the tool runs, the orchestrator:

- capability-matches each role to one of the identity-only agents
- builds a per-role `AgentSpecializationBundle`
- wraps each bundle in TDF with a policy bound to the assigned agent's DID
- stages it on the Iroh data plane and signals the agent
- launches the SwarmFlight and dispatches the first per-role task

After a bundle is applied, the previously identity-only agent reports
its assigned persona via `agent_discover`: purpose, model, and MCP
tool grants come from the bundle, not from `identity-only.swarmkit.yaml`.
