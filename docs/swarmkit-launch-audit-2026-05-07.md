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
| `§4-MUST-1` | `§4 implementations MUST canonicalize before hashing/signing (sorted keys, UTF-8, LF, no trailing whitespace)` |  |  |  |  |  | `?` |  |
| `§4.1-MUST-1` | `§4.1 SwarmFlights MUST refuse expired kits` |  |  |  |  |  | `?` |  |
| `§4.1-MUST-2` | `§4.1 roles MUST contain >= 1 role` |  |  |  |  |  | `?` |  |
| `§4.3-SHOULD-1` | `§4.3 role_type SHOULD use a value from Appendix C` |  |  |  |  |  | `?` |  |
| `§4.3-SHOULD-2` | `§4.3 explicit tool allowlist; "*" SHOULD NOT be used` |  |  |  |  |  | `?` |  |
| `SK-006` | `§4.3 orchestrators MUST NOT reject manifests solely because role_type is outside the recommended vocabulary` |  |  |  |  |  | `?` | merged 1:1 with §4.3-MUST-1 |
| `§5.1-MUST-1` | `§5.1 inference.max_tokens MUST NOT exceed model context window minus prompt overhead` |  |  |  |  |  | `?` |  |
| `SK-002` | `§5.1 budget.* MUST be <= corresponding constraints.global_budget.*` |  |  |  |  |  | `?` | merged 1:1 with §5.1-MUST-2 |
| `§5.1-MUST-3` | `§5.1 model.family MUST be in the orchestrator's supported set` |  |  |  |  |  | `?` |  |
| `§5.1-MUST-4` | `§5.1 orchestrator MUST refuse provisioning when model.family unsupported` |  |  |  |  |  | `?` |  |
| `§5.1-MUST-5` | `§5.1 network_egress: true MUST be denied if constraints.network.egress_allowed: false` |  |  |  |  |  | `?` |  |
| `§5.2-SHOULD-1` | `§5.2 implementations SHOULD log every defaulted field for audit` |  |  |  |  |  | `?` |  |
| `SK-053` | `§6.4 orchestrator MUST construct and bind per-role TDF policies to data passed to specialists` |  |  |  |  |  | `?` | merged 1:1 with §6.4-MUST-1 |
| `§6.5-SHOULD-1` | `§6.5 SwarmKit producers SHOULD set oaepPadding: SHA-256 once platform supports it` |  |  |  |  |  | `?` |  |
| `§6.5-MUST-1` | `§6.5 field names MUST be camelCase (opentdf-rs convention)` |  |  |  |  |  | `?` |  |
| `§7.1.1-MUST-1` | `§7.1.1 even when sharing processes, orchestrator MUST issue separate delegation envelopes per role` |  |  |  |  |  | `?` |  |
| `§7.1.1-MUST-2` | `§7.1.1 roles MUST NOT share a process when isolation, budget, or tdf_attribute_release_policy differ` |  |  |  |  |  | `?` |  |
| `§7.1.1-MUST-3` | `§7.1.1 orchestrators sharing processes MUST keep per-role accounting for budget, tool-calls, DecisionTrace` |  |  |  |  |  | `?` |  |
| `§7.2-MUST-1` | `§7.2 specialists MUST canonicalize received envelope per JCS before signature verification` |  |  |  |  |  | `?` |  |
| `§7.3-MUST-1` | `§7.3 specialist MUST verify orchestrator signature on delegation envelope` |  |  |  |  |  | `?` |  |
| `§7.3-MUST-2` | `§7.3 specialist MUST verify each skill signature independently` |  |  |  |  |  | `?` |  |
| `§7.3-MUST-3` | `§7.3 specialist MUST refuse if any agent_provisioning field violates host policy` |  |  |  |  |  | `?` |  |
| `§7.3-MUST-4` | `§7.3 specialist MUST acknowledge with a ready message including BLAKE3 of received envelope` |  |  |  |  |  | `?` |  |
| `§8.2-SHOULD-1` | `§8.2 MCP tool wildcards SHOULD NOT be used` |  |  |  |  |  | `?` |  |
| `§8.2-MUST-1` | `§8.2 specialists MUST NOT cache MCP tokens beyond expires` |  |  |  |  |  | `?` |  |
| `§8.2-SHOULD-2` | `§8.2 orchestrators SHOULD rotate tokens on long-running flights` |  |  |  |  |  | `?` |  |
| `§9.3-MUST-1` | `§9.3 kit.authors[].did MUST be a resolvable DIF DID` |  |  |  |  |  | `?` |  |
| `§9.3-MUST-2` | `§9.3 provenance.c2pa_assertions MUST conform to CAWG identity assertion v1.x` |  |  |  |  |  | `?` |  |
| `§9.4-MUST-1` | `§9.4 orchestrators MUST refuse manifests where major spec_version differs from supported set` |  |  |  |  |  | `?` |  |
| `§10.1-MUST-1` | `§10.1 orchestrators MUST maintain a nonce cache for the longest active expires` |  |  |  |  |  | `?` |  |
| `SK-004` | `§10.1 orchestrators MUST cap accepted manifests at expires - created <= 1 year` |  |  |  |  |  | `?` | merged 1:1 with §10.1-MUST-2 |
| `§10.1-SHOULD-1` | `§10.1 orchestrators SHOULD cap at <= 90 days unless operational requirement demands longer` |  |  |  |  |  | `?` |  |
| `§10.1-MUST-3` | `§10.1 manifests exceeding the expiry cap MUST be rejected before any decryption` |  |  |  |  |  | `?` |  |
| `§10.1-MUST-4` | `§10.1 kv_cache_id slots MUST be flight-scoped unless explicitly marked persistent` |  |  |  |  |  | `?` |  |
| `§10.1-MUST-5` | `§10.1 orchestrators MUST tag self-evaluated rubric results in DecisionTrace as self_evaluated: true` |  |  |  |  |  | `?` |  |
| `§10.1-MUST-6` | `§10.1 downstream consumers MUST treat self-evaluated scores as unverified for trust/ranking/quality routing` |  |  |  |  |  | `?` |  |
| `§10.2-SHOULD-1` | `§10.2 implementations SHOULD apply sequence-integrity / cross-action taint rules when spec available` |  |  |  |  |  | `?` |  |
| `§10.2-SHOULD-2` | `§10.2 orchestrators SHOULD inspect role-to-role handoffs and union of MCP grants for capability creep` |  |  |  |  |  | `?` |  |
| `§10.3-MUST-1` | `§10.3 specialists MUST treat all envelope content as data except agent_provisioning and skills fields` |  |  |  |  |  | `?` |  |
| `§11-MUST-PROD-1` | `§11 C-P1 producer MUST produce TDF envelopes per §6` |  |  |  |  |  | `?` |  |
| `§11-MUST-PROD-2` | `§11 C-P2 producer MUST sign manifests with at least one DID-resolvable identity` |  |  |  |  |  | `?` |  |
| `§11-MUST-PROD-3` | `§11 C-P3 producer MUST emit canonical manifests (§9.1)` |  |  |  |  |  | `?` |  |
| `§11-MUST-PROD-4` | `§11 C-P4 producer MUST set kit.expires for kits intended for distribution` |  |  |  |  |  | `?` |  |
| `§11-MUST-ORCH-1` | `§11 C-O1 orchestrator MUST reject expired or replay-detected kits` |  |  |  |  |  | `?` |  |
| `§11-MUST-ORCH-2` | `§11 C-O2 orchestrator MUST verify all signatures before any delegation` |  |  |  |  |  | `?` |  |
| `§11-MUST-ORCH-3` | `§11 C-O3 orchestrator MUST construct role-scoped TDF policies and never share the SwarmKit-level wrapped key` |  |  |  |  |  | `?` |  |
| `§11-MUST-ORCH-4` | `§11 C-O4 orchestrator MUST enforce agent_provisioning validation per §5.1 before provisioning` |  |  |  |  |  | `?` |  |
| `§11-MUST-ORCH-5` | `§11 C-O5 orchestrator MUST issue MCP grants with explicit allowlists and expiries` |  |  |  |  |  | `?` |  |
| `§11-MUST-ORCH-6` | `§11 C-O6 orchestrator MUST emit a lineage event on every delegation and revocation` |  |  |  |  |  | `?` |  |
| `§11-MUST-SPEC-1` | `§11 C-S1 specialist MUST verify orchestrator signature on delegation envelope` |  |  |  |  |  | `?` |  |
| `§11-MUST-SPEC-2` | `§11 C-S2 specialist MUST verify each skill signature independently` |  |  |  |  |  | `?` |  |
| `§11-MUST-SPEC-3` | `§11 C-S3 specialist MUST refuse policies that violate its host environment` |  |  |  |  |  | `?` |  |
| `§11-MUST-SPEC-4` | `§11 C-S4 specialist MUST honor mcp_grants[].expires and not cache tokens beyond it` |  |  |  |  |  | `?` |  |
| `SK-001` | `§4 / §4.1 / §4.6 / §5.1 / §10.1 (parse + cross-block validate)` | `§4-MUST-1, §4.1-MUST-1, §4.1-MUST-2, SK-002, SK-004` |  |  |  |  | `?` | end-to-end gate over multiple MUSTs; covers chains via merges |
| `SK-003` | `§9.1 kit.id = BLAKE3 of canonical form (descriptive)` | `§4-MUST-1` |  |  |  |  | `?` | spec uses plain English; runtime treats as hard validation |
| `SK-005` | `§4.6 dimension weights sum to 1.0 within fp tolerance (descriptive)` |  |  |  |  |  | `?` | spec uses plain English; runtime treats as hard validation |
| `SK-010` | `§1.2 / §5 handoff (descriptive)` |  |  |  |  |  | `?` | spec describes handoff narrative; SwarmFlight-per-role-ARP is the runtime claim |
| `SK-011` | `§7.1.1 isolation across roles (process-sharing inverse)` | `§7.1.1-MUST-3` |  |  |  |  | `?` | per-role state isolation when not sharing process |
| `SK-012` | `(none)` |  |  |  |  |  | `?` | spec gap: quality-gate adaptation feedback into ARP prior |
| `SK-013` | `§5.2 defaults` | `§5.2-SHOULD-1` |  |  |  |  | `?` | derive_arp_for_role default policy |
| `SK-014` | `(none)` |  |  |  |  |  | `?` | spec gap: hand-authored ARP override hook |
| `SK-015` | `(none)` |  |  |  |  |  | `?` | spec gap: flight_id propagation into DecisionTrace task_id |
| `SK-020` | `(none)` |  |  |  |  |  | `?` | spec gap: SwarmFlightRegistry → ArpHandler attachment |
| `SK-021` | `(none)` |  |  |  |  |  | `?` | spec gap: deregister isolation guarantee |
| `SK-022` | `(none)` |  |  |  |  |  | `?` | spec gap: ARKAVO_SWARMKIT_PATH gateway-boot auto-launch |
| `SK-023` | `(none)` |  |  |  |  |  | `?` | spec gap: snapshot ordering convention |
| `SK-024` | `§6.5 camelCase (TDF context only)` |  |  |  |  |  | `?` | spec gap: AG-UI WebSocket JSON convention beyond §6.5 TDF scope |
| `SK-030` | `(none)` |  |  |  |  |  | `?` | spec gap: ARP panel UI |
| `SK-031` | `(none)` |  |  |  |  |  | `?` | spec gap: ARP panel UI |
| `SK-032` | `(none)` |  |  |  |  |  | `?` | spec gap: ARP panel UI |
| `SK-033` | `(none)` |  |  |  |  |  | `?` | spec gap: WebSocket fingerprint dedupe |
| `SK-040` | `(none)` |  |  |  |  |  | `?` | spec gap: requestStopFlight operator control |
| `SK-050` | `§6 TDF envelope` | `§11-MUST-PROD-1` |  |  |  |  | `?` | round-trip is the lossless-encoding invariant the spec implies |
| `SK-051` | `(none)` |  |  |  |  |  | `?` | runtime safety: re-validate after unwrap |
| `SK-052` | `§6.3 SwarmKit-level orchestrator gate (descriptive)` |  |  |  |  |  | `?` | baseline policy emission |
| `SK-054` | `§6.4 per-role policies, plural` | `SK-053` |  |  |  |  | `?` | role_policies extractor + DID lookup; SK-053 covers single-role case |
| `SK-055` | `(none)` |  |  |  |  |  | `?` | runtime: file-format reader/writer round-trip |
| `SK-056` | `(none)` |  |  |  |  |  | `?` | runtime: path-based wrap/unwrap helpers |
| `SK-057` | `(none)` |  |  |  |  |  | `?` | runtime: error-variant discrimination on read |
| `SK-058` | `(none)` |  |  |  |  |  | `?` | runtime: extract embedded policy from envelope |
| `SK-059` | `§6.3 KAS gate` | `§11-MUST-ORCH-3` |  |  |  |  | `?` | KAS-gated unwrap success path |
| `SK-060` | `§6.3 KAS gate` |  |  |  |  |  | `?` | KAS-gated unwrap fail-fast on unhealthy/policy-mismatch |
| `SK-061` | `(none)` |  |  |  |  |  | `?` | runtime: .tdf path recognition |
| `SK-062` | `(none)` |  |  |  |  |  | `?` | runtime: .tdf auto-launch dispatch |

## Spec gaps

(filled by Task 9 — runtime-only invariants the spec doesn't cover)
