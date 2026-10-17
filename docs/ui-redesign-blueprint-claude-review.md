# Review: Arkavo Operations web UI redesign blueprint

Reviewer: Claude Opus 5.5 (`claude-opus-5-5`) via Claude CLI.

Review date: October 17, 2026.

Reviewed document: [UI redesign blueprint](ui-redesign-blueprint.md).

Input SHA-256: `bf230ad296beb3b894913c4e4fa11d5a1ea99e0fcd7d12652e31815e866e025c`.

Review method: document-only review with tools disabled. The report below preserves Claude's review text; repository behavior and external claims were not independently verified by Claude. The blueprint was left unchanged.

Scope: this review covers only the supplied blueprint text. I had no access to repository files, specs, the design concept, or external sites. I have not verified any claim about source code, spec counts, browser walkthroughs or competitor documentation, and I ran no tests. Claims that depend on those sources are listed separately under "Source-verification questions".

## Overall verdict

**Approve the direction, but revise before implementation starts beyond the Foundation phase.**

The blueprint is unusually disciplined about claims. It consistently separates "existing", "runtime capability" and "proposed" behavior. It refuses composite trust scores and treats transport success as distinct from command acceptance. These positions are correct, and they are rare in documents of this kind.

The weaknesses are sequencing gaps and missing decisions, not overstated claims:

- **Attributed approvals arrive late.** The Operations workspace phase ships Decisions and the "Operator / authority" header before authenticated operators exist.
- **Command semantics are only half-defined.** The blueprint promises "Outcome unknown" reconciliation, but the gateway it describes cannot answer that query until the Durable operation phase.
- **Freshness has no thresholds.** "Stale" thresholds, priority ranking, and the trust assumption placed on the gateway itself are all undefined.
- **Two contracts are missing.** Mission "Validating" and "Cancelled" states have no admission or cancel contracts behind them.

**Critical document-level issues: none found.** No section makes a security or readiness claim that the document itself contradicts. No section instructs implementers to build a control that would mislead operators. The highest severity below is **High**, and several High items are blocking decisions for phases 2–3.

## Confirmed document issues (internal contradictions and omissions)

### D1 — High — Decisions ships before operator authentication

- **Where:**
  - "Operations workspace: expose existing value" ships Decisions and has the exit condition "review an existing ARP proposal".
  - "Durable operation" is the first phase that implements "authenticated operator sessions and scoped grants".
- **Conflict:** "Review a proposed change" says "Approval binds the reviewer". The Critical finding says proposal review records the fixed string `agui-operator`. The Overview wireframe shows "Operator / authority" in the top bar.
- **Problem:** Phase 2 would present a redesigned approval flow and an operator/authority header while approvals are still unattributed.
- **Correction:** State the interim rule explicitly. Recommended rule:
  - In phase 2, label ARP decisions as "gateway-local, unattributed".
  - Show no operator identity in the header; show "Operator: not authenticated" instead.
  - Exclude Decisions from any multi-operator deployment until phase 3.
  - Alternatively, move ARP approve/reject into phase 3 and ship Decisions read-only in phase 2.

### D2 — High — Phase 1 promises reconciliation that the phase 1 gateway cannot provide

- **What phase 1 must do:** "Create and run a mission" requires that "A dropped reply produces **Outcome unknown** … Recovery queries by the same idempotency key". Acceptance requires "retry cannot duplicate execution".
- **What phase 1 has:** The findings note that "Gateway task tracking is an in-memory map". Idempotency and receipts only arrive in "Durable operation". The Foundation exit condition promises only "cannot … silently discard an action".
- **Problem:** The blueprint does not say what the operator can do with an "Outcome unknown" task during phases 1–2, or after a gateway restart.
- **Correction:** Define the interim behavior. Recommended:
  - "Outcome unknown" disables retry for that draft.
  - Offer "copy as new task" with an explicit duplicate-risk warning.
  - Map each acceptance row to the phase that must satisfy it (see D9).

### D3 — High — Mission states imply missing capabilities

- **"Validating":** The state path includes "Validating" and "Show the effective execution locality, tool/data permissions, budget ceilings and stop conditions before dispatch".
  - But the findings state "the gateway selects an agent and delegates; decomposition depends on that agent".
  - The contracts table has no admission or dry-run contract that could compute effective permissions before dispatch.
- **"Cancelled":** The state path includes "Cancelled", yet the document states that `requestStopFlight` "does not cancel remote work". No cancel command or cancel receipt appears in the contracts.
- **Correction:**
  - Add an "Admission preview" contract, with an explicit "not resolvable before dispatch" outcome.
  - Add a "Cancel request / cancel receipt" contract that has per-target acknowledgment, as containment does.
  - Until both exist, drop "Validating" and "Cancelled" from the phase 1–2 lifecycle.

### D4 — High — Freshness vocabulary has no thresholds, owner or clock basis

- **Undefined terms:**
  - "Persistent operational context" requires "Current, Stale …" and "observation age".
  - The Foundation phase must "introduce per-source freshness".
  - The doc never says who defines the Stale threshold per source, or what clock observation age is computed against.
- **Partial coverage:** The degraded table mentions "clock uncertainty excessive" without defining a threshold.
- **Correction:**
  - Add freshness policy (threshold, clock source, uncertainty bound) to the Capability descriptor or Snapshot envelope, so the backend supplies it rather than the browser.
  - State that the browser computes age from server-supplied observed/received times, not from its own clock alone.

### D5 — High — The trust assumption on the gateway is unstated

- **What the threat model covers:** "Contested" includes "potentially compromised workers". The doc rules out containing a compromised host administrator.
- **What it omits:** All security evidence, receipts and containment acknowledgments reach the browser through the gateway. The doc never says whether the gateway is trusted. "Security evidence … never browser-generated" leaves gateway-generated or gateway-relayed evidence unaddressed.
- **Correction:** State the gateway's trust level explicitly. Either:
  - (a) the gateway is inside the trusted computing base, and that limitation is shown in Evidence; or
  - (b) receipts are signed by enforcing components and verified independently of the gateway.

  The answer decides whether "Containment receipt" needs signatures and an issuer field.

### D6 — Medium — Overview ranking depends on an undefined priority model

- **Where it is used:**
  - Overview C "ranks by consequence and deadline".
  - The Overview primary action is "Open the highest-priority actionable item".
  - The operator study targets "highest-priority blocker within 10 seconds".
- **Problem:** No consequence model, deadline source or tie-break rule is defined. The 10-second study target cannot be scored until one is.
- **Correction:**
  - Define priority inputs and who supplies them: backend-supplied, or derived in documented browser selectors.
  - Require each ranked item to display why it ranks where it does.

### D7 — Medium — State vocabularies overlap and are not unified

- **The doc defines six separate sets:**
  - Context: "Current, Stale, Unknown, Unavailable, Denied, Unsupported".
  - Trust: "Not reported".
  - Security state: "Unassessed, Eligible, Suspended, Quarantined".
  - Coverage: "Supported and enforced, Configured but unverified, Not configured, Unsupported, Unknown".
  - Containment: "requested, enforced, failed or unreachable".
  - Empty states: a sixth set of its own.
- **Overlaps:** "Not reported", "Unknown" and "Unassessed" overlap. "Denied" mixes an authorization outcome into a freshness/availability list.
- **Correction:** Add one glossary that keeps orthogonal axes separate: freshness, availability, authorization result, coverage, security state and acknowledgment. Define each term once and state which axis it belongs to.

### D8 — Medium — Policy has no single canonical home

- **The split:** "Agent Runtime Policy" maps to "Decisions; Missions → Flights; Data & Trust → Policy".
- **The overlap:** Security & Data Plane maps to "Data & Trust; related records in Evidence". It is unclear whether, for example, TDF audit lives in Data & Trust or in Evidence.
- **Correction:** Add an object-to-canonical-workspace table, with one owner per object type. Other workspaces show excerpts and link to the owner. This also makes "Deep links restore scope and selected record" implementable.

### D9 — Medium — Acceptance gates are not mapped to phases, and some are not testable

- **No phase mapping:** The acceptance table is not tied to delivery phases. For example, "Containment requested on partitioned fleet" can only apply to phase 4.
- **Untestable rows:** "Main workflows usable" has no workflow list or assistive-technology matrix.
- **Unmeasurable study:** The operator study has no participant definition, baseline or method for measuring "mistaken confidence".
- **Correction:**
  - Add a phase column to the table.
  - Enumerate the "main workflows".
  - Name the screen readers and browsers to be tested.
  - Define mistaken confidence, for example as a self-rated confidence score on incorrect answers.

### D10 — Medium — Readiness labels are defined but never applied

- **Where:** "Readiness terms used here" defines "Existing UI path / Runtime capability / Proposed".
- **Problem:** No table or workflow is tagged with these labels.
- **Correction:** Add a readiness column to the "Current functionality", "Differentiators" and workflow sections. This is the cheapest single change for keeping implementers honest.

### D11 — Medium — Scope and multi-gateway model are undecided

- **Where:** The wireframe has a "Scope" selector, and the Snapshot envelope includes "Source instance, scope".
- **Problem:** The architecture describes a single embedded gateway. The doc never decides whether one console can span multiple gateways or swarms, or what defines "scope".
- **Correction:** Decide single-gateway for this redesign, or define scope semantics. Identifier stability and deep links depend on the answer.

### D12 — Medium — Degraded-link traffic needs a subscription contract

- **Where:** "Continue during degraded connectivity" asks to "coalesce diagnostic metrics" and to "Avoid high-frequency full snapshots for every hidden panel".
- **Problem:** No contract covers subscription, backpressure or priority classes, so the behavior cannot be implemented as written.
- **Correction:** Add a "Subscription/priority" contract with:
  - visible-scope subscriptions;
  - priority classes (receipts and security first);
  - a server-side coalescing interval;
  - a gap marker when data is dropped.

### D13 — Medium — Bounded audit retention conflicts with reconstruction and handover

- **Where:** The Audit event contract says "persist with bounded retention". "Reconstruct an outcome" and "authorized handover export" depend on that history.
- **Correction:**
  - Define a retention owner and a minimum retention period.
  - State how expired history is shown, for example as "retention gap" rather than "collection gap".

### D14 — Low — Accessibility targets have gaps

- **Zoom level:** "Essential status must remain visible at 200% zoom" is weaker than WCAG reflow, which requires 320 CSS px (equivalent to 400%).
- **Focus obscured:** A persistent status bar risks violating 2.4.11 Focus Not Obscured.
- **Target size:** 2.5.8 Target Size is not mentioned.
- **Announcements:** The scope of "Announce consequential status changes" is undefined.
- **Correction:**
  - Test at 320 px and at 400% zoom.
  - Ensure the sticky header collapses rather than obscuring focus.
  - List which event types use live regions, with a rate limit.

### D15 — Low — Process wording and secondary scope are unclear

- **Unclear wording:** "stack subsequent topics from creation if split" cannot be acted on without clarification.
- **CEF renderer:** It is "outside this first web redesign", but the doc does not say whether CEF will consume the same versioned contracts.
- **Correction:** Clarify the wording, and state whether the CEF renderer will consume the same contracts.

## Product recommendations (judgment, not document defects)

- **P1 — High — The headline positioning leans on the least-complete capability.** The proposed line "with visible boundaries on … where data can go" depends on egress mediation. The doc itself lists that as incomplete ("NET-003, NET-016, NET-019 and NET-020 … egress and reply mediation"). Two options:
  - Lead with what is enforced today: local-first routing with explicit cloud escalation, and permit-bound tool invocation.
  - Or attach "where mediated" to the data-boundary claim until coverage reporting ships.
- **P2 — Medium — Add an "Open decisions" section with an owner per item.** Most of the D-findings are decisions without owners. Collecting them in one place makes blocking status visible.
- **P3 — Medium — Treat the contextual assistant as an injection surface.** "Mission detail" allows an assistant to "propose a next step" while agent output is untrusted. State that:
  - assistant-proposed commands are shown as proposals tied to the assistant's sources;
  - the operator must confirm each one;
  - proposals never pre-fill privileged fields.
- **P4 — Medium — Cover session lifecycle.** Phase 3 needs rules for session expiry mid-review, re-authentication before privileged actions, and multiple tabs per operator (draft and idempotency scope).
- **P5 — Low — Define portion and page marking rules for classification.** "Classification markings come from authoritative metadata" does not say how a page that aggregates records with different markings (Overview, Evidence) is marked. Decide this before any classified deployment profile.
- **P6 — Low — Name the competitors the wedge is measured against.** The "What is already common" section addresses general frameworks and disconnected-ops platforms. A short comparison against the alternatives the target teams actually evaluate would make the wedge testable.

## Source-verification questions

These depend on material I could not see. Each should be checked before the blueprint is circulated.

- **V1 — Inventory counts.** Verify "91 component spec files, 978 scenario-ID entries and 105 `wip: true` markers", and the claim that `specs/README.md` and `FUTURE.md` are out of date.
- **V2 — Current-behavior claims.** Verify each claim that underlies a Critical or High finding:
  - `wsSend` drops sends silently while `submitTask` closes the composer;
  - `requestStopFlight` only removes registry/ARP entries;
  - the fixed `agui-operator` reviewer string;
  - the in-memory task map;
  - zero/boolean substitution in `security.js`;
  - console-only approval failures;
  - the fixed power/price energy defaults.
- **V3 — Spec status claims.** Verify:
  - permit-dispatch verification of issuer, PoP, arguments, time and budget;
  - ARP blast-radius checks, and that mesh-wide changes cannot auto-apply;
  - router local-only behavior and cloud-spend authorization being "implemented in relevant paths".
- **V4 — Proposed targets.** Confirm that the 5-second lease and 1-second p99 containment targets match `swarm-trust-model.md`. Check how a 5-second lease interacts with "already-authorized local processing may continue" during link loss longer than the lease.
- **V5 — Engineering targets.** Confirm the source of the ≤50 ms router and ≤65 MB binary targets.
- **V6 — Competitor documentation.** Confirm the LangGraph, Microsoft Agent Framework and Palantir Apollo characterizations against the linked pages as of the review date.
- **V7 — Design concept testing.** Confirm the desktop/320 px Chrome checks and the scenario-switching test of the design concept. The concept was not supplied with this document.

## Strengths

- **Honest framing.** The blueprint repeatedly separates proposed controls from enforced ones ("not a declaration of operational security readiness"; "UI badges alone do not qualify this milestone").
- **Correct command semantics.** Transport send is not acceptance, there is no silent replay, "Outcome unknown" is a first-class state, and the browser never buffers privileged commands.
- **Precise containment model.** Detection, assessment, containment request and enforced containment are separate facts. Acknowledgments are per enforcement point, and a reachable gateway never implies that other nodes have stopped.
- **Sound trust presentation.** Reachability, identity, membership, authority, assurance, security state, coverage and freshness are kept separate. The blueprint refuses composite scores and shows "Allowed for this action" rather than a global allowed badge.
- **Strong degraded-connectivity table.** It handles concrete failure modes, including gateway restart reconciliation and agents that disappear without being marked completed.
- **Concrete failure-oriented acceptance scenarios.** These include malicious agent text, stale approvals and partitioned containment. The operator study targets mistaken confidence, not just speed.
- **Grounded market framing.** It acknowledges commoditized capabilities and lists phrases to avoid.

## Top recommended blueprint changes

1. **Approvals interim rule (D1).** Add an explicit rule for approvals before authentication, or move ARP approve/reject to the Durable operation phase.
2. **Phase 1–2 command behavior (D2, D3).** Define "Outcome unknown" handling for phases 1–2. Add Admission-preview and Cancel contracts, or remove "Validating" and "Cancelled" until they exist.
3. **Gateway trust (D5).** State the gateway trust assumption, and whether receipts are signed by enforcing components.
4. **Freshness ownership (D4).** Put freshness thresholds and clock basis into a backend-supplied contract.
5. **Unified state glossary (D7).** Publish one glossary with orthogonal axes.
6. **Canonical object homes (D8).** Add an object-to-canonical-workspace table.
7. **Phase-mapped, testable acceptance (D9, D10).** Map every acceptance scenario to a phase, apply the readiness labels throughout, and make the accessibility and operator-study gates measurable.
8. **Missing contracts (D12, D13).** Add a subscription/priority contract and an audit retention decision.
9. **Positioning (P1).** Re-anchor positioning on currently enforced capabilities until egress coverage reporting exists.
