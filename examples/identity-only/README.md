# Identity-only mesh fixtures

Three minimal roles (`agent-0`, `agent-1`, `agent-2`) in a single kit,
`identity-only.swarmkit.yaml`, that declare nothing but identity, listen
address, and capability hints. The orchestrator's `apply-kit` flow uses
these as the pool of agents that get hyperspecialized into roles when a
SwarmKit is applied.

There is no purpose, no model selection, no MCP tool list, and no API
tokens until the orchestrator ships a TDF-wrapped
`AgentSpecializationBundle` to the agent over A2A.

## Demo

```bash
# 1. Start the three identity-only agents (each in its own terminal).
cargo run -p arkavo -- agent -c examples/identity-only/identity-only.swarmkit.yaml -n agent-0 -p 8341
cargo run -p arkavo -- agent -c examples/identity-only/identity-only.swarmkit.yaml -n agent-1 -p 8342
cargo run -p arkavo -- agent -c examples/identity-only/identity-only.swarmkit.yaml -n agent-2 -p 8343

# 2. Start the orchestrator agent.
cargo run -p arkavo -- agent -c examples/orchestrator-agent/orchestrator-agent.swarmkit.yaml -n orchestrator -p 8340

# 3. Apply the Campaign Kit. The orchestrator will:
#    - capability-match each role to one of the identity-only agents
#    - build a per-role AgentSpecializationBundle
#    - wrap each bundle in TDF with a policy bound to the assigned agent's DID
#    - ship via agent.specialize over A2A
#    - launch the SwarmFlight and dispatch the first per-role task
cargo run -p arkavo -- orchestrator apply-kit \
  examples/campaign-kit/campaign-kit.swarmkit.yaml
```

After apply-kit returns, each previously identity-only agent reports
its assigned persona via `agent_discover` — purpose, model, and MCP
tool grants come from the bundle, not from `identity-only.swarmkit.yaml`.
