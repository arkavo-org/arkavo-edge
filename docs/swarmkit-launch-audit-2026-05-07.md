# SwarmKit Ground-Truth Audit (2026-05-07)

phase: SwarmKit Launch Plan, Phase 1
spec for this audit: docs/superpowers/specs/2026-05-07-swarmkit-launch-audit.md
exit gate for: Phase 2 (closing credibility gaps)

## Externally-claimed surface

### Runtime invariants (from `specs/arkavo-edge/swarmkit.spec.yaml` invariants block)

1. SwarmKit manifest is parser+validator only — no runtime side effects from parsing.
2. `kit.id` equals BLAKE3 of the canonical manifest with `kit.id` and `provenance.signatures` stripped.
3. `kit.expires - kit.created` cannot exceed 1 year (spec §10.1 cap).
4. `evaluation.rubric.dimensions` weights sum to 1.0 within 1e-6 tolerance (spec §4.6).
5. `role_type` is free-form per spec §4.3 / Appendix C; the orchestrator does not reject domain-specific values.
6. `SwarmFlight::launch` builds one `ArpRuntime` per role (spec §1.2 / §5 handoff).
7. Each role's `DecisionTrace` and `PolicyCache` are isolated from other roles in the same flight.
8. Per-role `budget.max_total_tokens` cannot exceed `constraints.global_budget.max_total_tokens` when validated (spec §5.1).
9. `ArpHandler` keys flight roles under synthetic agent_id `flight:<flight_id>:<role_id>` so multi-flight role_id collisions are impossible.
10. `FlightContext` on `AgentArpStatus` carries `kit_id`, `kit_name`, `role_id`, `role_type` for the AG-UI panel.
11. `ARKAVO_SWARMKIT_PATH` auto-launch failures are logged and non-fatal — a misconfigured kit does not prevent gateway boot.

### Runtime scenarios (from `SK-NNN` in `swarmkit.spec.yaml`, treating each as a runtime claim)

12. TDF envelope round-trips a SwarmKit manifest losslessly with a SwarmKit-level orchestrator policy (SK-050).
13. KAS-gated unwrap fails fast on unhealthy KAS or on policy attributes missing — distinct from KAS-side denial during decrypt (SK-060).

### Validate flow (from `examples/campaign-kit/README.md`)

14. The campaign-kit example validates by running `cargo run -p arkavo-swarmkit --example validate_kit -- examples/campaign-kit/campaign-kit.swarmkit.yaml`, which parses, validates cross-block invariants, and computes the BLAKE3 `kit.id`.

### Current disclaimers (input to ship-blocker rule clause 3)

- `examples/campaign-kit/README.md` states the MVP defers (a) TDF encryption envelope and per-role attribute release policies, (b) A2A JSON-RPC delegation envelope (spec §7.2), and (c) Creator UI / approval screen, all to follow-up PRs.
- Top-level `README.md` contains no SwarmKit mentions (verified by `grep -in swarmkit README.md`).

_Frozen 2026-05-07 — no additions for the rest of this audit._

## Ship-blocker punch list

count: TBD
(filled by Task 12 — bulleted list of `ship_blocker=Y` rows linked by id)

## Audit table

| id | spec_ref | covers | label | confidence | evidence | owner | ship_blocker | notes |
|----|----------|--------|-------|------------|----------|-------|--------------|-------|

## Spec gaps

(filled by Task 9 — runtime-only invariants the spec doesn't cover)
