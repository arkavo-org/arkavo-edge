# ARP Showcase

Boots the AG-UI web gateway with an Agent Runtime Policy (ARP) document loaded so the new **Agent Runtime Policy** panel renders real data.

The included `arkavo.arp.json` is a fully-validated ARP 0.1.0 document covering all four enforcement layers (cognitive, execution, data sovereignty, network) plus adaptation, feedback loops, budget, escalation, session, HITL, state storage, and observability sections.

## Running

```bash
# from repo root
cargo build
./examples/arp-showcase/run.sh
```

Then open <http://127.0.0.1:7700> and click the scales icon in the left navigation.

## What you should see

- **Agent** — selector at the top, since each agent in the mesh owns its own ARP. The gateway's locally-loaded document appears as `(local)`. As real agents join the mesh and report their policies via A2A, they will populate this list. The dropdown shows the document status and any violation count per agent (e.g. `rover-alpha (OK) [3 viol]`).
- **Document** — VALID status, ADL URI, ARP spec version, sections-present chips for every section configured in the document.
- **Violations** — filtered view of decision traces where `outcome.success == false` or `event_type ∈ {quarantine, escalation, budget_event, hitl_action}`. Empty until the runtime emits enforcement events. This is the single source of truth for "what got blocked"; the Security panel will cross-link here.
- **Policy Cache (runtime)** — empty-state placeholder. The policy cache config (TTL, decay strategy, half-life) is read from the document and shown in the Document section, but live cache state populates here only once the agent loop starts inserting entries.
- **Adaptation Engine** — empty-state placeholder for the same reason. Beta priors will appear once the conductor drives entity selection.
- **Recent Decision Traces** — full trace stream (success and denied) for the selected agent. Empty until the runtime emits trace entries.

## Pointing at a different document

The handler reads `ARKAVO_ARP_PATH`. Override before running:

```bash
ARKAVO_ARP_PATH=/path/to/your.arp.json ./examples/arp-showcase/run.sh
```

If neither `ARKAVO_ARP_PATH` nor a `./arkavo.arp.json` is present, the panel shows `NOT LOADED` with no document body.

## Verifying invalid documents

To see how the panel reports errors, edit `arkavo.arp.json` to break the JSON or remove a required field. The status flips to `INVALID` and the `Error` row shows the parser/validator message.
