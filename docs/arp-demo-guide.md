# ARP Demo Guide

Agent Runtime Policy is a critical, unique feature of Arkavo Edge: each agent in the mesh carries its own policy, enforces it locally, and emits a signed audit trail. This guide is the playbook for demonstrating it to engineering, security, or product audiences.

## Framing — the unique pitch

Open with the contrast:

> *"Most agent platforms put policy on the server side. We put it on the agent — every agent in the mesh carries its own ARP, enforces it locally, and emits a signed audit trail. That changes the failure mode: there's no central policy server to compromise, no single bottleneck to rate-limit you, and policies are heterogeneous by design."*

Three differentiators that aren't on competitors' slides:

- **Per-agent self-governance.** The mesh dropdown is the demo. Three agents, three policies, three budgets. No central policy server.
- **Bayesian adaptation.** Every enforcement decision feeds a Beta prior; policies *learn* which tools, peers, and models are reliable. Prior/posterior state is visible in the trace.
- **DecisionTrace as audit-grade evidence.** Ed25519-signed entries, append-only, per ARP §17.1. Not a log file — a cryptographic record.

## Audience-specific framings

- **Security / CISO.** Lead with violations, audit trace, signed entries, per-agent isolation. The ratchet-tighten / human-relax model.
- **Engineering.** Lead with the per-agent dropdown, the document validation at load, and the observability hooks (`attach_policy_cache`, `attach_decision_trace`).
- **Product / sales.** Lead with the multi-pane operations console — ARP is one of ten panels, all live-streaming.

## Setup

Five minutes before the demo:

```bash
cargo build
./examples/arp-showcase/run.sh
```

Open <http://127.0.0.1:7700> and click the scales icon (⚖) in the left navigation.

The showcase document at `examples/arp-showcase/arkavo.arp.json` exercises all four enforcement layers (cognitive, execution, data sovereignty, network) plus adaptation, feedback loops, budget, escalation, session, state storage, and observability. The Document section tells a story on its own.

For multi-agent realism: before the demo, wire two extra agents into the mesh by setting `ARKAVO_ARP_PATH` in their startup scripts (or push via `ArpHandler::set_agent_arp` once an A2A `arp.snapshot` method is wired).

## Demo flow — 8 to 10 minutes

### Beat one — "what's a policy in Arkavo?" (90 seconds)

Open the ARP panel on `(local)`. Walk the Document section top-to-bottom:

- **VALID status + path.** "This was parsed and validated against the ARP schema at startup. A bad policy never reaches the runtime."
- **ARP Spec / ADL URI / ADL Hash / Integrity Signed.** "The policy is cryptographically bound to its companion ADL document. Tamper either, signature breaks."
- **Adaptation `thompson_sampling`.** "This agent makes routing decisions probabilistically, not by static rule."
- **Quality Gate `0.7 / composite`.** "Every action below quality 0.7 updates the prior. Self-correcting."
- **Policy Cache `TTL 3600s / exponential decay / human-exempt yes`.** "Lessons decay unless a human teaches them. Stops policy drift over time."
- **Sections chips.** "Ten sections active. Cognitive (PII redaction), Execution (sandbox), Network (egress)..."

### Beat two — "each agent has its own" (60 seconds)

Pause and switch the dropdown:

- **`rover-alpha`.** "Different budget — $0.10 vs $2.50 — tighter quality threshold, has escalation rules. This rover handles hazard detection so it's locked down harder."
- **`rover-beta`.** INVALID, with the parse error showing. "Caught at policy-load. The rover never started with a broken policy."

This is the moment that lands the per-agent point.

### Beat three — "watch it block something" (90 seconds)

This requires runtime enforcement wired into the conductor. Two options:

- **Live (preferred, requires wiring).** From the Tasks panel submit a task that calls `shell.exec` or hits a non-allowlisted URL. Switch back to the ARP panel — the agent dropdown shows `rover-alpha [1 viol]`, then `[2 viol]`, etc. Open the agent and walk the Violations table: time, layer, event, denied reason, task ID.
- **Scripted today.** Inject violations via the browser console for the demo:

  ```js
  AppState.arpStatus = { agents: [...], timestamp: new Date().toISOString() };
  AppState.arpSelectedAgent = 'rover-alpha';
  renderArp();
  ```

  The screenshot at `examples/arp-showcase/arp-multi-agent-violations.png` is your fallback if a live demo isn't possible.

Talk track on the violations table:

> *"Each row is a denied or escalated action. Layer tells you which boundary refused it — `network` egress, `execution` budget, `cognitive` drift. Reason is human-readable. Task ID is the cross-link to the run that tried it. This is the SOC view: every refused action is an audit event, not a swallowed error."*

### Beat four — "it's also evidence" (60 seconds)

Scroll to **Recent Decision Traces** below Violations. Same data, but unfiltered — successes too. Point out the `ok` vs `denied` column.

> *"Auditors don't just want to see the failures. They want proof that 99 things ran cleanly and one was blocked."*

Mention that with the `signing` feature flag each entry is Ed25519-signed; with `cryptographic_signing.signing_required_above_sensitivity: confidential` you get tamper-evident logs that meet GRC retention rules.

### Beat five — "and it learns"

The runtime is now driving both engines. Each tool invocation in the conductor's tool loop updates two things:

- The PolicyCache writes a hash-chained, decay-tracked entry keyed by `tool.outcome.<name>.<n>`.
- The AdaptationEngine updates the Beta prior for that tool — successes above the quality gate increase `alpha`, failures or below-gate outcomes increase `beta`.

Show this in the panel: under `Policy Cache (runtime)` the status row reads **live -- updated by the conductor on every tool outcome**. Under `Adaptation Engine` the priors table fills in as the agent calls tools.

Talk track:

> *"Every tool outcome feeds the Beta prior. After 20 successful invocations, the prior shows alpha 21, beta 1, mean 0.95 — the agent strongly trusts that tool. Failures push beta up. The cache logs each verdict with a decay curve. Lessons fade unless reinforced. Human-taught lessons never decay."*

## The kill shot

If you have time, end with this. Switch to the **Security & Data Plane** panel.

> *"This is the SOC view — KAS, TDF audit, data plane. Notice the violation count badge: it links back to ARP. Same data, two lenses. Engineers debug in ARP, security teams monitor in Security."*

The cross-link is a small follow-up addition; the data path is already live.

## What not to show — be honest

- **Per-agent A2A push not yet implemented.** Today the gateway loads its own document as `(local)`. Real per-agent documents need a JSON-RPC `arp.snapshot` method. The handler API (`ArpHandler::set_agent_arp`, `attach_policy_cache`, `attach_decision_trace`) is ready; the transport isn't.
- **Hot-reload.** Editing the ARP file doesn't reload it — the gateway loads once at startup. Easy to add but not done.
- **Standalone showcase has no agent.** `./examples/arp-showcase/run.sh` boots only the UI, so the cache and adaptation tables are live but empty. To see populated state, run a real agent process (which embeds the conductor) and let it call tools — the gateway picks up the same `ArpRuntime` via the process-global registry.

## Q&A prep

- *"Why JSON?"* — Schema-validated, human-diffable, signable, language-agnostic. ADL is a separate document on purpose.
- *"How do you stop an agent from lying about its policy?"* — That's why ADL hash and Ed25519 integrity exist. The mesh refuses to peer with agents whose advertised policy doesn't match its signed ADL.
- *"What if a policy is too restrictive and blocks legitimate work?"* — `escalation.relaxation_requires: human_approval`. The ratchet only tightens automatically; loosening requires a HITL signature.
- *"Comparison to OPA / Rego?"* — OPA is centralized and stateless. ARP is per-agent and stateful (priors, decay, gossip). Different problem class.
- *"What about latency?"* — Policy evaluation is in-process. PolicyCache is `DashMap`-backed concurrent hash map. Hash-chain integrity is checked on demand, not on every read.
- *"What happens during gossip?"* — `feedback_loops.gossip` controls peer propagation. Trusted peers' lessons enter the local cache at a discount factor; untrusted DIDs at zero weight until probation passes.

## Timing guide

- **8–10 minutes** is the sweet spot. Run the full beat sequence.
- **15 minutes.** Add the Beat 5 deep-dive (priors, decay curves, gossip propagation).
- **5 minutes.** Drop Beat 1 (document walkthrough). Lead with the multi-agent dropdown — that's the unique thing.

## Reference materials

- `examples/arp-showcase/` — runnable showcase, sample policy, screenshots.
- `crates/arkavo-arp/` — parser, model, validator. 65+ tests.
- `crates/arkavo-policy-cache/` — runtime cache with hash-chain integrity, decay strategies.
- `crates/arkavo-adaptation/` — Thompson Sampling, ε-greedy, UCB1 selection.
- `crates/arkavo-observability/src/decision_trace.rs` — append-only audit log, Ed25519 signing.
- `crates/arkavo-agui/src/arp_handler.rs` — gateway-side per-agent state aggregator.
- `crates/arkavo-agui/static/js/panels/arp.js` — the UI panel rendered in the demo.
