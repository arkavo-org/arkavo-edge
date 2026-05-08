# SwarmKit `apply-kit` manual test plan

Operator-side verification for PR #589 (SwarmKit orchestrator-driven role
binding with TDF specialization bundles). Automated unit and integration
tests cover the wire format and role-binding pipeline; this plan covers
the human-visible surfaces those tests cannot exercise.

Run from a clean checkout of `feature/swarmkit-orchestrator-apply` (or
`main` once merged). Set `ARKAVO_DEBUG=1` for richer logs.

## Prerequisites

- `cargo build -q` completes in the workspace root.
- A modern browser pointed at the gateway URL printed by `arkavo ui`.
- Two free TCP ports for the gateway WS + at least three more for the
  identity-only agents in M4 (defaults below assume 8341–8343 + 8340 for
  the orchestrator).

---

## M1 — Gateway surfaces auto-launch failures

**Goal:** Confirm a misconfigured `ARKAVO_SWARMKIT_PATH` produces a
visible "SwarmKit auto-launch failed" entry in the ARP panel rather
than disappearing into the log.

### Steps

```bash
ARKAVO_DEBUG=1 ARKAVO_SWARMKIT_PATH=/tmp/does-not-exist.swarmkit.yaml \
  cargo run -p arkavo --bin arkavo -- ui
```

In a second terminal, hit the gateway WS to capture the `ArpStatusUpdate`
snapshot directly (the ARP panel polls every 5 s, so an automated probe
is faster than reloading the browser):

```bash
# Find the ws port from the gateway log line "AG-UI gateway listening on …"
GATEWAY_PORT=…
echo '{"type":"requestArpStatus"}' \
  | websocat -n1 ws://127.0.0.1:${GATEWAY_PORT}/ws \
  | jq '.snapshot.swarmkitLaunchErrors'
```

### Expected

- `swarmkitLaunchErrors` is a one-element array with `kind == "read"`,
  `path` matching `/tmp/does-not-exist.swarmkit.yaml`, and `message`
  containing the OS-level "No such file" text.
- Gateway log carries a `WARN` line `failed to auto-launch SwarmFlight`.
- Browser ARP panel shows a "SwarmKit auto-launch failed" section above
  Active SwarmFlights with the same path/message in `status-warn` red.

### Pass criteria

`swarmkitLaunchErrors[0].kind == "read"` AND the panel renders the row.

---

## M2 — Successful auto-launch leaves errors empty

**Goal:** Confirm the same code path stays out of the way when the kit
loads cleanly.

### Steps

```bash
ARKAVO_DEBUG=1 ARKAVO_SWARMKIT_PATH=$(pwd)/examples/campaign-kit/campaign-kit.swarmkit.yaml \
  cargo run -p arkavo --bin arkavo -- ui
```

Then poll the snapshot the same way as M1.

### Expected

- `snapshot.swarmkitLaunchErrors` is `[]`.
- `snapshot.agents` contains three flight-role entries, each with
  `flightContext.kitName == "Campaign Kit"`.
- Browser panel shows "Active SwarmFlights" with three role pills
  (analyst / copy / critic), `boundAgentDid` is absent (auto-launch is
  unbound).

### Pass criteria

Panel shows the three role pills AND no launch-error section.

---

## M3 — CLI subcommand smoke test

**Goal:** Confirm `arkavo orchestrator apply-kit` is wired up, surfaces
useful errors when the orchestrator is unreachable, and does not panic
on bad input.

### Steps

```bash
# 3.1 — help text renders
cargo run -p arkavo --bin arkavo -- orchestrator apply-kit --help

# 3.2 — unreachable orchestrator
cargo run -p arkavo --bin arkavo -- orchestrator apply-kit \
  examples/campaign-kit/campaign-kit.swarmkit.yaml \
  --orchestrator-url http://127.0.0.1:1

# 3.3 — missing manifest path
cargo run -p arkavo --bin arkavo -- orchestrator apply-kit \
  /tmp/missing.swarmkit.yaml
```

### Expected

- 3.1 shows the `--orchestrator-url` flag and `MANIFEST` argument,
  exit 0.
- 3.2 exits non-zero with a clear "POST … failed" message naming the
  unreachable URL — not a panic, not a generic JSON error.
- 3.3 exits non-zero with `resolve manifest path …: No such file or
  directory` — error before any network round trip.

### Pass criteria

All three exit cleanly with the expected diagnostics; no panics.

---

## M4 — Identity-only mesh + apply-kit (full vertical, best-effort)

**Goal:** Drive the end-to-end "user picks a kit, orchestrator
specializes the swarm" flow against three identity-only agents.

This is the highest-value pass and the one most likely to surface
remaining wiring gaps (KAS availability, orchestrator JSON-RPC method
registration, A2A specialize handler reachability). Document what works
and what's blocked.

### Setup (4 terminals)

```bash
# T1 — agent-0 (advertises summarize, asset-store)
cd $(pwd)/examples/identity-only/agent-0 \
  && ARKAVO_DEBUG=1 cargo run -p arkavo --bin arkavo -- agent --verbose

# T2 — agent-1 (write, social-publisher)
cd $(pwd)/examples/identity-only/agent-1 \
  && ARKAVO_DEBUG=1 cargo run -p arkavo --bin arkavo -- agent --verbose

# T3 — agent-2 (critique, scoring)
cd $(pwd)/examples/identity-only/agent-2 \
  && ARKAVO_DEBUG=1 cargo run -p arkavo --bin arkavo -- agent --verbose

# T4 — orchestrator agent
cd $(pwd)/examples/orchestrator-agent \
  && ARKAVO_DEBUG=1 cargo run -p arkavo --bin arkavo -- agent --verbose
```

### Action

```bash
# T5
cargo run -p arkavo --bin arkavo -- orchestrator apply-kit \
  examples/campaign-kit/campaign-kit.swarmkit.yaml
```

### Expected

- CLI prints a JSON assignment plan: `bindings[]` with one entry per
  manifest role, each pointing at one of agent-0..2 by DID.
- Each agent's log shows a successful `agent.specialize` RPC and a
  `Specialization applied` line.
- Each agent's `agent_discover` (probe via JSON-RPC) reports the
  role-specific `purpose` from the bundle, not the empty AGENTS.md
  value.
- Gateway ARP panel shows three role pills with the bound DID short-form
  appended.
- Each role's DecisionTrace gains at least one `tool_outcome` entry
  within 60 s (proves the first task dispatched).

### Known unknowns (blocked-OK)

- The orchestrator agent does **not** yet expose
  `orchestrator.apply_kit` as a JSON-RPC method — `apply_kit` is a Rust
  function. Wiring is a follow-up. If the CLI errors with
  `JSON-RPC error: Method not found`, that is the expected blocker for
  this slice.
- `BundleDecryptor` defaults to `UnconfiguredBundleDecryptor` on the
  agent side. Without a KAS-backed decryptor wired into the agent
  binary, every `agent.specialize` call returns `-32603 "Bundle
  decryption failed: agent has no bundle decryptor configured"`. That
  is also the expected blocker; production wiring is a follow-up.

### Pass criteria

The full chain runs end-to-end **OR** the test cleanly hits one of the
two known blockers above with the predicted error string. Either is an
acceptable outcome for this slice; surprise failures are not.

---

## Results

| ID | Date | Tester | Result | Notes |
|----|------|--------|--------|-------|
| M1 | 2026-05-02 | claude (auto) | pass | WS snapshot: `swarmkitLaunchErrors[0] = {kind: "read", message: "No such file or directory (os error 2)", path: "/tmp/does-not-exist.swarmkit.yaml"}`. Gateway started cleanly on port 7799. |
| M2 | 2026-05-02 | claude (auto) | pass | 3 flight roles registered (analyst, copy, critic) with `kitName: "Campaign Kit (3-agent MVP)"`; `swarmkitLaunchErrors == []`; `boundAgentDid` absent (auto-launch is unbound, expected). |
| M3 | 2026-05-02 | claude (auto) | pass | (3.1) help shows `--orchestrator-url` flag and `MANIFEST` arg, exit 0. (3.2) `Error: POST http://127.0.0.1:1: error sending request`, exit 1. (3.3) `Error: resolve manifest path /tmp/missing.swarmkit.yaml: No such file or directory`, exit 1 — short-circuits before any network round trip. |
| M4 | 2026-05-02 | claude (auto) | blocked (as predicted) | 3 identity-only agents started cleanly on 8341/8342/8343 and discovered each other via mDNS. `agent_discover` confirmed empty `purpose`/`model`/MCP tools — true identity-only state. `apply-kit` POST to a reachable agent endpoint returned exactly the predicted blocker: `JSON-RPC error: -32601 Method not found`, because `orchestrator.apply_kit` is not yet wired as an A2A method. The bundle-decryptor blocker would have followed if the JSON-RPC call had landed. Both follow-ups are tracked in this slice's "Out of scope" section. |

### Follow-up issues to file

1. **Register `orchestrator.apply_kit` as an A2A JSON-RPC method on the orchestrator agent.** Current state: the Rust function exists in `arkavo-orchestrator::apply_kit`; the orchestrator agent's `A2aRpcImpl` does not yet expose it. Without this, the CLI subcommand cannot drive the pipeline against a live orchestrator.
2. **Wire a KAS-backed `BundleDecryptor` into the agent binary.** Current state: `A2aServer::new` initializes `bundle_decryptor` to `UnconfiguredBundleDecryptor`; production code must call `set_bundle_decryptor` with an `OpenTdfService`-backed implementation before `start_agent_server` boots the RPC.
3. **Expose AGENTS.md `capabilities:` field.** The identity-only fixtures declare `capabilities: summarize, asset-store` etc., but `parse_agents_config` does not currently parse that key. The `register_agent` call from the agent-side mDNS discovery would need to forward those strings into `AgentRegistry` so `RoleCapabilityMatcher::match_role` has something to match on. Until then, even with #1 and #2 done, the matcher would fall back to `RoleUncoverable`.

---

## Cleanup

```bash
# Kill any leftover gateway / agent processes.
pkill -f 'target/debug/arkavo' || true
```
