# ARP Showcase

Boots the AG-UI web gateway with an Agent Runtime Policy (ARP) document loaded so the **Agent Runtime Policy** panel renders real data.

The included `arkavo.arp.json` is a fully-validated ARP 0.1.0 document covering all four enforcement layers (cognitive, execution, data sovereignty, network) plus adaptation, feedback loops, budget, escalation, session, HITL, state storage, and observability sections.

A companion `arkavo.swarmkit.yaml` declares a single-role SwarmKit so the same panel can render an **Active SwarmFlights** section alongside the rich `(local)` ARP agent. This is how the SwarmKit per-role ARP handoff (spec §1.2 / §5) appears in the UI — derived ARP runtime per role, isolated DecisionTrace, deregister via the panel's Stop button.

## Running

```bash
# from repo root
cargo build

# Original showcase: just the (local) ARP agent.
./examples/arp-showcase/run.sh

# Showcase + SwarmKit: (local) ARP agent + an Active SwarmFlight section.
./examples/arp-showcase/run-swarmkit.sh
```

Then open <http://127.0.0.1:7700> and click the scales icon in the left navigation.

## What you should see

- **Agent** — selector at the top, since each agent in the mesh owns its own ARP. The gateway's locally-loaded document appears as `(local)`. As real agents join the mesh and report their policies via A2A, they will populate this list. The dropdown shows the document status and any violation count per agent (e.g. `rover-alpha (OK) [3 viol]`).
- **Document** — VALID status, ADL URI, ARP spec version, sections-present chips for every section configured in the document.
- **Violations** — filtered view of decision traces where `outcome.success == false` or `event_type ∈ {quarantine, escalation, budget_event, hitl_action}`. Empty until the runtime emits enforcement events. This is the single source of truth for "what got blocked"; the Security panel will cross-link here.
- **Policy Cache (runtime)** — live. The handler instantiates a `PolicyCache` from the loaded document and the panel shows entry count, hash-chain validity, plus a per-entry table once the conductor records tool outcomes. Standalone `arkavo ui` shows zero entries because there's no agent invoking tools; run a full agent process to see the cache fill.
- **Adaptation Engine** — live. The handler instantiates an `AdaptationEngine` and exposes Beta priors per tool entity. Each conductor tool call updates the prior — `alpha` grows on success above the quality gate, `beta` grows on failure or below-gate outcomes.
- **Recent Decision Traces** — full trace stream (success and denied) for the selected agent. The runtime emits a `tool_invocation` entry per tool call with prior/posterior Beta state, latency, quality score, and an error type when an outcome is below the quality gate or fails outright. Standalone `arkavo ui` shows zero entries because there's no agent invoking tools.

## Active SwarmFlights section

When `ARKAVO_SWARMKIT_PATH` is set (as `run-swarmkit.sh` does), the gateway also auto-launches a SwarmFlight at boot. The showcase kit declares one role (`showcase`, `role_type: showcase_agent`), and the panel renders it as a clickable pill above the agent dropdown. Click the pill to inspect that role's derived ARP runtime; click the **Stop** button to drop the flight without disturbing the `(local)` ARP agent.

## Pointing at a different document

The handler reads `ARKAVO_ARP_PATH` and `ARKAVO_SWARMKIT_PATH`. Override before running:

```bash
ARKAVO_ARP_PATH=/path/to/your.arp.json ./examples/arp-showcase/run.sh
ARKAVO_SWARMKIT_PATH=/path/to/your.swarmkit.yaml ./examples/arp-showcase/run-swarmkit.sh
```

If neither `ARKAVO_ARP_PATH` nor a `./arkavo.arp.json` is present, the panel shows `NOT LOADED` with no document body.

## Verifying invalid documents

To see how the panel reports errors, edit `arkavo.arp.json` to break the JSON or remove a required field. The status flips to `INVALID` and the `Error` row shows the parser/validator message.
