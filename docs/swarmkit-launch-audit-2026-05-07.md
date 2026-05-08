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
| `§4-MUST-1` | `§4 implementations MUST canonicalize before hashing/signing (sorted keys, UTF-8, LF, no trailing whitespace)` |  | wired | high | `crates/arkavo-swarmkit/src/canonical.rs:22-58` |  | N | JCS RFC 8785 implementation; exercised via SK-003 |
| `§4.1-MUST-1` | `§4.1 SwarmFlights MUST refuse expired kits` |  | aspirational | high | `no test coverage` |  | `?` | confirmed: SwarmFlight::launch (flight.rs:111-161) only checks empty roles + override resolution. No current-time expiry check at any layer. Pass 2b confirmed Pass 2a's medium call |
| `§4.1-MUST-2` | `§4.1 roles MUST contain >= 1 role` |  | wired | high | `crates/arkavo-swarmkit/src/validate.rs:101-103` |  | N | NoRoles error variant |
| `§4.3-SHOULD-1` | `§4.3 role_type SHOULD use a value from Appendix C` |  | aspirational | high | `no test coverage` |  | `?` | producer guidance, parser intentionally accepts any string (SK-006 inverts this); not enforced |
| `§4.3-SHOULD-2` | `§4.3 explicit tool allowlist; "*" SHOULD NOT be used` |  | aspirational | high | `no test coverage` |  | `?` | wildcard guard not enforced anywhere in arkavo-swarmkit |
| `SK-006` | `§4.3 orchestrators MUST NOT reject manifests solely because role_type is outside the recommended vocabulary` |  | wired | high | `crates/arkavo-swarmkit/src/role.rs:8-9` |  | N | merged 1:1 with §4.3-MUST-1; `role_type: String` deserializes any value |
| `§5.1-MUST-1` | `§5.1 inference.max_tokens MUST NOT exceed model context window minus prompt overhead` |  | aspirational | high | `no test coverage` |  | `?` | not in validate.rs; orchestrator-side at provisioning time. No code path in this crate |
| `SK-002` | `§5.1 budget.* MUST be <= corresponding constraints.global_budget.*` |  | wired | high | `crates/arkavo-swarmkit/src/validate.rs:158-192 + tests:333-349` |  | N | merged 1:1 with §5.1-MUST-2; covers max_total_tokens and max_wallclock_ms |
| `§5.1-MUST-3` | `§5.1 model.family MUST be in the orchestrator's supported set` |  | aspirational | high | `no test coverage` |  | `?` | orchestrator concern; arkavo-swarmkit accepts any model.family string |
| `§5.1-MUST-4` | `§5.1 orchestrator MUST refuse provisioning when model.family unsupported` |  | aspirational | high | `no test coverage` |  | `?` | orchestrator concern, not parser |
| `§5.1-MUST-5` | `§5.1 network_egress: true MUST be denied if constraints.network.egress_allowed: false` |  | wired | high | `crates/arkavo-swarmkit/src/validate.rs:194-207 + tests:351-361` |  | N | NetworkEgressDenied error variant |
| `§5.2-SHOULD-1` | `§5.2 implementations SHOULD log every defaulted field for audit` |  | aspirational | high | `no test coverage` |  | `?` | derive.rs uses DeriveOptions defaults silently (no tracing); confirmed across parser + runtime |
| `SK-053` | `§6.4 orchestrator MUST construct and bind per-role TDF policies to data passed to specialists` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:186-245` |  | N | merged 1:1 with §6.4-MUST-1; role_policy + role_policies functions, attribute splitting, DID dissemination |
| `§6.5-SHOULD-1` | `§6.5 SwarmKit producers SHOULD set oaepPadding: SHA-256 once platform supports it` |  | aspirational | medium | `no test coverage` |  | `?` | depends on arkavo-tdf default; not driven by SwarmKit producer code |
| `§6.5-MUST-1` | `§6.5 field names MUST be camelCase (opentdf-rs convention)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs (TdfManifest from arkavo_tdf) + tests:write_kit_tdf_round_trips_through_reader` |  | N | TDF envelope fields inherit camelCase from arkavo_tdf::TdfManifest serialization; round-trip test passes Pass 3a |
| `§7.1.1-MUST-1` | `§7.1.1 even when sharing processes, orchestrator MUST issue separate delegation envelopes per role` |  | aspirational | high | `no test coverage` |  | `?` | process-sharing optimization not implemented; no delegation envelope code anywhere (grep confirms) |
| `§7.1.1-MUST-2` | `§7.1.1 roles MUST NOT share a process when isolation, budget, or tdf_attribute_release_policy differ` |  | aspirational | high | `no test coverage` |  | `?` | depends on §7.1.1 implementation which doesn't exist |
| `§7.1.1-MUST-3` | `§7.1.1 orchestrators sharing processes MUST keep per-role accounting for budget, tool-calls, DecisionTrace` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/flight.rs:189-207 + tests:263-292 (SK-011)` |  | N | per-role isolation invariant exercised by SK-011 (Pass 3a green) even though process-sharing itself isn't implemented |
| `§7.2-MUST-1` | `§7.2 specialists MUST canonicalize received envelope per JCS before signature verification` |  | aspirational | high | `no test coverage` |  | `?` | no specialist-side delegation envelope code in arkavo-edge; entire §7.2 is the gap the launch plan calls out |
| `§7.3-MUST-1` | `§7.3 specialist MUST verify orchestrator signature on delegation envelope` |  | aspirational | high | `no test coverage` |  | `?` | no specialist code in this repo |
| `§7.3-MUST-2` | `§7.3 specialist MUST verify each skill signature independently` |  | aspirational | high | `no test coverage` |  | `?` | no skill-resolver/verifier code anywhere — Skill type is a manifest field only |
| `§7.3-MUST-3` | `§7.3 specialist MUST refuse if any agent_provisioning field violates host policy` |  | aspirational | high | `no test coverage` |  | `?` | no specialist-side host-policy check |
| `§7.3-MUST-4` | `§7.3 specialist MUST acknowledge with a ready message including BLAKE3 of received envelope` |  | aspirational | high | `no test coverage` |  | `?` | no specialist code |
| `§8.2-SHOULD-1` | `§8.2 MCP tool wildcards SHOULD NOT be used` |  | aspirational | high | `no test coverage` |  | `?` | wildcard guard not enforced; same as §4.3-SHOULD-2 from a different angle |
| `§8.2-MUST-1` | `§8.2 specialists MUST NOT cache MCP tokens beyond expires` |  | aspirational | high | `no test coverage` |  | `?` | no specialist-side MCP token cache in arkavo-edge |
| `§8.2-SHOULD-2` | `§8.2 orchestrators SHOULD rotate tokens on long-running flights` |  | aspirational | high | `no test coverage` |  | `?` | no orchestrator-side MCP token rotation |
| `§9.3-MUST-1` | `§9.3 kit.authors[].did MUST be a resolvable DIF DID` |  | aspirational | high | `no test coverage` |  | `?` | parser accepts any string in `did` field (Author struct in manifest.rs); no DID resolution check |
| `§9.3-MUST-2` | `§9.3 provenance.c2pa_assertions MUST conform to CAWG identity assertion v1.x` |  | aspirational | high | `no test coverage` |  | `?` | parser accepts c2pa_assertions as opaque; no CAWG validation |
| `§9.4-MUST-1` | `§9.4 orchestrators MUST refuse manifests where major spec_version differs from supported set` |  | wired | high | `crates/arkavo-swarmkit/src/validate.rs:89-99` |  | N | semver-parsed major != 1 → SpecVersionMismatch error |
| `§10.1-MUST-1` | `§10.1 orchestrators MUST maintain a nonce cache for the longest active expires` |  | aspirational | high | `no test coverage` |  | `?` | orchestrator-side persistence concern; not in arkavo-swarmkit or runtime |
| `SK-004` | `§10.1 orchestrators MUST cap accepted manifests at expires - created <= 1 year` |  | wired | high | `crates/arkavo-swarmkit/src/validate.rs:209-223 + tests:423-439` |  | N | merged 1:1 with §10.1-MUST-2; covers ExpiryHorizonTooLarge + ExpiryBeforeCreated |
| `§10.1-SHOULD-1` | `§10.1 orchestrators SHOULD cap at <= 90 days unless operational requirement demands longer` |  | aspirational | high | `no test coverage` |  | `?` | constant `RECOMMENDED_EXPIRY_HORIZON_SECONDS` defined in validate.rs:17 but never enforced |
| `§10.1-MUST-3` | `§10.1 manifests exceeding the expiry cap MUST be rejected before any decryption` |  | wired | low | `crates/arkavo-swarmkit/src/validate.rs:209-223` |  | N | trivially true by construction: validate() runs on parsed manifest before any decrypt path. No explicit test asserts the temporal ordering |
| `§10.1-MUST-4` | `§10.1 kv_cache_id slots MUST be flight-scoped unless explicitly marked persistent` |  | aspirational | medium | `no test coverage` |  | `?` | KV cache exists in arkavo-kv-cache; flight-scoping not enforced from SwarmKit side. Owner needed for verification |
| `§10.1-MUST-5` | `§10.1 orchestrators MUST tag self-evaluated rubric results in DecisionTrace as self_evaluated: true` |  | aspirational | high | `no test coverage` |  | `?` | no DecisionTrace tag for self_evaluated; evaluation block parses but tagging not implemented |
| `§10.1-MUST-6` | `§10.1 downstream consumers MUST treat self-evaluated scores as unverified for trust/ranking/quality routing` |  | aspirational | high | `no test coverage` |  | `?` | depends on §10.1-MUST-5 tag which doesn't exist; no consumer-side handling |
| `§10.2-SHOULD-1` | `§10.2 implementations SHOULD apply sequence-integrity / cross-action taint rules when spec available` |  | aspirational | high | `no test coverage` |  | N | spec itself notes "When a sequence-integrity specification… is available" — that spec doesn't exist yet, so this SHOULD is conditional and not blocking |
| `§10.2-SHOULD-2` | `§10.2 orchestrators SHOULD inspect role-to-role handoffs and union of MCP grants for capability creep` |  | aspirational | high | `no test coverage` |  | `?` | parser validates handoffs resolve, but no capability-creep analysis |
| `§10.3-MUST-1` | `§10.3 specialists MUST treat all envelope content as data except agent_provisioning and skills fields` |  | aspirational | high | `no test coverage` |  | `?` | no specialist code in this repo |
| `§11-MUST-PROD-1` | `§11 C-P1 producer MUST produce TDF envelopes per §6` | `SK-050` | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:88-98 (wrap_manifest)` |  | N | producer wrap path covered by SK-050 |
| `§11-MUST-PROD-2` | `§11 C-P2 producer MUST sign manifests with at least one DID-resolvable identity` |  | aspirational | medium | `no test coverage` |  | `?` | manifest carries provenance.signatures field but no signing helper in arkavo-swarmkit; parser accepts unsigned manifests |
| `§11-MUST-PROD-3` | `§11 C-P3 producer MUST emit canonical manifests (§9.1)` | `§4-MUST-1, SK-003` | wired | high | `crates/arkavo-swarmkit/src/canonical.rs:22-58` |  | N | producers using arkavo-swarmkit emit canonical via canonical_json; verified via SK-003 round-trip |
| `§11-MUST-PROD-4` | `§11 C-P4 producer MUST set kit.expires for kits intended for distribution` |  | aspirational | high | `no test coverage` |  | N | manifest's `expires` is `Option<String>`; parser allows None. Producer guidance, not enforced |
| `§11-MUST-ORCH-1` | `§11 C-O1 orchestrator MUST reject expired or replay-detected kits` | `§4.1-MUST-1, §10.1-MUST-1` | aspirational | high | `no test coverage` |  | `?` | depends on §4.1-MUST-1 and §10.1-MUST-1, both aspirational |
| `§11-MUST-ORCH-2` | `§11 C-O2 orchestrator MUST verify all signatures before any delegation` |  | aspirational | high | `no test coverage` |  | `?` | no signature-verification code path; provenance.signatures parses as opaque |
| `§11-MUST-ORCH-3` | `§11 C-O3 orchestrator MUST construct role-scoped TDF policies and never share the SwarmKit-level wrapped key` | `SK-053, SK-054, SK-059` | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:186-245` |  | N | role_policy / role_policies build per-role policies; SwarmKit-level vs role-scoped policies are separate functions |
| `§11-MUST-ORCH-4` | `§11 C-O4 orchestrator MUST enforce agent_provisioning validation per §5.1 before provisioning` | `SK-002` | wired | high | `crates/arkavo-swarmkit/src/validate.rs:88-156` |  | N | validate() runs before SwarmFlight::launch can construct ARP runtimes |
| `§11-MUST-ORCH-5` | `§11 C-O5 orchestrator MUST issue MCP grants with explicit allowlists and expiries` |  | aspirational | high | `no test coverage` |  | `?` | McpToolGrant struct exists in role.rs but no grant-issuance code path |
| `§11-MUST-ORCH-6` | `§11 C-O6 orchestrator MUST emit a lineage event on every delegation and revocation` |  | aspirational | high | `no test coverage` |  | `?` | DecisionTrace exists per-role but no lineage stream of delegation/revocation events |
| `§11-MUST-SPEC-1` | `§11 C-S1 specialist MUST verify orchestrator signature on delegation envelope` |  | aspirational | high | `no test coverage` |  | `?` | duplicate of §7.3-MUST-1 |
| `§11-MUST-SPEC-2` | `§11 C-S2 specialist MUST verify each skill signature independently` |  | aspirational | high | `no test coverage` |  | `?` | duplicate of §7.3-MUST-2; the entire Skill resolver is the launch-plan gap |
| `§11-MUST-SPEC-3` | `§11 C-S3 specialist MUST refuse policies that violate its host environment` |  | aspirational | high | `no test coverage` |  | `?` | duplicate of §7.3-MUST-3 |
| `§11-MUST-SPEC-4` | `§11 C-S4 specialist MUST honor mcp_grants[].expires and not cache tokens beyond it` |  | aspirational | high | `no test coverage` |  | `?` | duplicate of §8.2-MUST-1 |
| `SK-001` | `§4 / §4.1 / §4.6 / §5.1 / §10.1 (parse + cross-block validate)` | `§4-MUST-1, §4.1-MUST-1, §4.1-MUST-2, SK-002, SK-004` | wired | high | `crates/arkavo-swarmkit/src/lib.rs:39-44 + manifest.rs:14-30` |  | N | end-to-end gate; covers chains via merges. Note: §4.1-MUST-1 (refuse expired) is not part of this end-to-end gate — it's a runtime check, not a parse-time check |
| `SK-003` | `§9.1 kit.id = BLAKE3 of canonical form (descriptive)` | `§4-MUST-1` | wired | high | `crates/arkavo-swarmkit/src/canonical.rs:92-100 + validate.rs:tests 449-465` |  | N | spec uses plain English; runtime treats as hard validation via KitIdHashMismatch |
| `SK-005` | `§4.6 dimension weights sum to 1.0 within fp tolerance (descriptive)` |  | wired | high | `crates/arkavo-swarmkit/src/validate.rs:140-146 + tests:393-421` |  | N | spec uses plain English; runtime treats as hard validation via RubricWeightsDoNotSumToOne |
| `SK-010` | `§1.2 / §5 handoff (descriptive)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/flight.rs:111-161 + tests:253-261` |  | N | spec describes handoff narrative; SwarmFlight-per-role-ARP is the runtime claim |
| `SK-011` | `§7.1.1 isolation across roles (process-sharing inverse)` | `§7.1.1-MUST-3` | wired | high | `crates/arkavo-swarmkit-runtime/src/flight.rs:189-207 + tests:263-292` |  | N | per-role state isolation when not sharing process |
| `SK-012` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/tests/campaign_kit_flight.rs:117-147 (below_quality_gate_outcome_degrades_role_prior)` |  | N | spec gap: quality-gate adaptation feedback into ARP prior; integration test passes Pass 3a |
| `SK-013` | `§5.2 defaults` | `§5.2-SHOULD-1` | wired | high | `crates/arkavo-swarmkit-runtime/src/derive.rs:76-130` |  | N | derive_arp_for_role default policy; constants documented in DeriveOptions |
| `SK-014` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/flight.rs:118-122 + tests:329-352` |  | N | spec gap: hand-authored ARP override hook; LaunchOptions.arp_overrides |
| `SK-015` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/flight.rs:200-202 + tests:294-312` |  | N | spec gap: flight_id propagation into DecisionTrace task_id |
| `SK-020` | `(none)` |  | wired | high | `crates/arkavo-agui/src/swarm_flight_registry.rs:36-56 + arp_handler.rs:204-235` |  | N | spec gap: SwarmFlightRegistry → ArpHandler attachment with FlightContext |
| `SK-021` | `(none)` |  | wired | high | `crates/arkavo-agui/src/swarm_flight_registry.rs:60-65 + arp_handler.rs:243-248` |  | N | spec gap: deregister isolation; idempotent on unknown flight_id |
| `SK-022` | `(none)` |  | wired | high | `crates/arkavo-agui/src/swarm_flight_registry.rs:91-102 + gateway.rs:401-417` |  | N | spec gap: ARKAVO_SWARMKIT_PATH gateway-boot auto-launch with non-fatal failure |
| `SK-023` | `(none)` |  | wired | high | `crates/arkavo-agui/src/arp_handler.rs:259-281` |  | N | spec gap: local → mesh → flight roles ordering convention |
| `SK-024` | `§6.5 camelCase (TDF context only)` |  | wired | high | `crates/arkavo-agui/src/types.rs:1126-1156` |  | N | spec gap: AG-UI WebSocket JSON convention beyond §6.5 TDF scope; flightContext field with skip_serializing_if for non-flight agents |
| `SK-030` | `(none)` |  | wired | high | `crates/arkavo-agui/static/js/panels/arp.js:81 (renderArpFlightsSection call) + swarmkit.spec.yaml:SK-030` |  | N | spec gap: ARP panel UI; JS-side, coverage via spec scenario contract not Rust unit test |
| `SK-031` | `(none)` |  | wired | high | `crates/arkavo-agui/static/js/panels/arp.js:91-101 + swarmkit.spec.yaml:SK-031` |  | N | spec gap: pill click → role selection; JS-side coverage via spec scenario |
| `SK-032` | `(none)` |  | wired | high | `crates/arkavo-agui/static/js/panels/arp.js:163-188 + swarmkit.spec.yaml:SK-032` |  | N | spec gap: pill status glyph (violations / traces / empty); JS-side coverage via spec scenario |
| `SK-033` | `(none)` |  | wired | high | `crates/arkavo-agui/static/js/panels/arp.js:13-35 + swarmkit.spec.yaml:SK-033` |  | N | spec gap: WebSocket fingerprint dedupe to preserve DOM state; JS-side coverage via spec scenario |
| `SK-040` | `(none)` |  | wired | high | `crates/arkavo-agui/src/gateway_ws.rs:351-385 + swarm_flight_registry.rs:60-65` |  | N | spec gap: requestStopFlight operator control with idempotent deregister + immediate ArpStatusUpdate |
| `SK-050` | `§6 TDF envelope` | `§11-MUST-PROD-1` | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:88-115 (wrap_manifest, unwrap_manifest)` |  | N | round-trip is the lossless-encoding invariant the spec implies |
| `SK-051` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:105-115` |  | N | runtime safety: unwrap_manifest pipes through parse_json which re-validates |
| `SK-052` | `§6.3 SwarmKit-level orchestrator gate (descriptive)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:132-144` |  | N | swarmkit_orchestrator_policy emits the §6.3 baseline gate |
| `SK-054` | `§6.4 per-role policies, plural` | `SK-053` | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:212-245` |  | N | role_policies extractor + DID lookup; SK-053 covers single-role case |
| `SK-055` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:269-289` |  | N | runtime: file-format reader/writer round-trip |
| `SK-056` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:309-336` |  | N | runtime: path-based wrap/unwrap helpers |
| `SK-057` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:291-307` |  | N | runtime: error-variant discrimination on read (Io vs Serialize) |
| `SK-058` | `(none)` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:282-326` |  | N | runtime: extract embedded policy from envelope |
| `SK-059` | `§6.3 KAS gate` | `§11-MUST-ORCH-3` | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:344-382` |  | N | KAS-gated unwrap success path; verified via test 824-833 |
| `SK-060` | `§6.3 KAS gate` |  | wired | high | `crates/arkavo-swarmkit-runtime/src/tdf.rs:355-365 + tests:840-865` |  | N | KAS-gated unwrap fail-fast on unhealthy/policy-mismatch; distinct from Decrypt error |
| `SK-061` | `(none)` |  | wired | high | `crates/arkavo-agui/src/swarm_flight_registry.rs:179-189` |  | N | runtime: .tdf path recognition (case-insensitive) |
| `SK-062` | `(none)` |  | wired | high | `crates/arkavo-agui/src/swarm_flight_registry.rs:104-117 + 140-170` |  | N | runtime: .tdf auto-launch dispatch via launch_from_tdf_path; TdfFeatureDisabled vs TdfUnwrap distinction |

## Spec gaps

Runtime invariants the spec does not address. Each is a candidate for a spec proposal in Phase 5 (or for the forthcoming RFC AE-2026-004).

**Note on scope.** A grep for `assert!`, `debug_assert!`, `panic!`, `unreachable!`, `unimplemented!` outside `#[cfg(test)]` modules across `arkavo-swarmkit`, `arkavo-swarmkit-runtime`, and the AG-UI integration files returned **zero hits**. SwarmKit's invariants are enforced at the type system, not via runtime panics. The gaps below are not panicking invariants but *behavioural runtime claims the test contract makes that the prose spec doesn't*.

### Spec-language vs runtime-validation gaps

The spec uses descriptive English where the runtime treats the claim as a hard invariant. Phase 5 should promote these to MUST clauses in the next spec draft.

- **§4.6 rubric weights sum to 1.0** — spec line 230 phrases it as "and the sum of weight across dimensions equals 1.0 (within floating-point tolerance)" without MUST. Runtime enforces via `RubricWeightsDoNotSumToOne` (validate.rs:140-146, exercised by SK-005). Recommendation: add `§4.6-MUST-1`.
- **§9.1 kit.id BLAKE3 formula** — spec line 516 phrases it as "kit.id is BLAKE3(canonical_manifest)" without MUST. Runtime enforces via `KitIdHashMismatch` (validate.rs:232-246, exercised by SK-003). Recommendation: add `§9.1-MUST-1`.
- **§1.2 / §5 handoff narrative** — spec describes the "initial conditions" → "ARP runtime" handoff descriptively. Runtime makes this concrete via `derive_arp_for_role` + `SwarmFlight::launch` (exercised by SK-010). Recommendation: §1.2 or §5 should add a MUST clause for "one ARP runtime per role at launch."

### Architecture-level gaps (worth a §3 architecture overview proposal)

- `flight:<flight_id>:<role_id>` synthetic-agent-id keying convention. Runtime claim from swarmkit.spec.yaml invariants: "ArpHandler stores flight roles under synthetic agent_id `flight:<flight_id>:<role_id>` so multi-flight role_id collisions are impossible." Implemented in `crates/arkavo-agui/src/arp_handler.rs:204-235` and `swarm_flight_registry.rs:36-56`. The spec doesn't define this keying scheme; it should, since any other implementation will collide on role_id.
- AG-UI WebSocket camelCase serialization for SwarmKit-derived events. Implemented in `crates/arkavo-agui/src/types.rs:1126-1156` with `flightContext` field. Spec only mandates camelCase in §6.5 (TDF cryptographic profile context); the same convention applied to AG-UI events is a separate undocumented runtime claim.
- `ArpStatusUpdate` fingerprint dedupe to preserve panel DOM state. Implemented in `arp.js:13-35`. This is a UI smoothing detail not in spec scope but materially affects observability and operator UX.

### Operator-control gaps (no spec section yet)

- `requestStopFlight` WebSocket command + `FlightStopped` event. Implemented in `gateway_ws.rs:351-385`. Spec §7.4 mentions revocation by closing A2A channels and KAS revocation events; the AG-UI operator-stop flow is a *different* surface (operator-initiated cancellation through the gateway WebSocket) that the spec doesn't describe.
- `ARKAVO_SWARMKIT_PATH` environment-driven auto-launch with non-fatal failure. Implemented in `swarm_flight_registry.rs:91-117`. Spec doesn't address how flights are started or what fault-tolerance gateway boot needs.

### Adaptation feedback gaps

- Quality-gate adaptation feedback into ARP prior. Implemented as `record_tool_outcome` in `flight.rs:189-207`, exercised by SK-012 (below-quality outcome degrades the role's prior). Spec §1.2 references the ARP companion spec for adaptation but doesn't pin the feedback contract.
- `LaunchOptions::arp_overrides` per-role hand-authored ARP. Implemented in `flight.rs:106-140`, exercised by SK-014. The spec implies derivation from `agent_provisioning` (§5.2 defaults) without acknowledging that orchestrators may need to bypass derivation for hand-authored ARP — a real production need.
- `flight_id` propagation into per-role DecisionTrace `task_id`. Implemented in `flight.rs:200-202`, exercised by SK-015. Spec doesn't define how flight identity surfaces in trace records, which matters for cross-flight observability.

### File-format / .swarmkit.tdf gaps

- `.swarmkit.tdf` recommended double-extension and case-insensitive recognition. Implemented in `swarm_flight_registry.rs:179-189` (SK-061). Spec Appendix B mentions filename conventions in passing; should be normative.
- Distinct `Io` vs `Serialize` vs `Decrypt` error variants on the read path. Implemented in `tdf.rs:291-307` (SK-057). Spec doesn't constrain error taxonomy; consumers benefit from being able to distinguish them.
- `TdfFeatureDisabled` vs `TdfUnwrap` auto-launch error variants. Implemented in `swarm_flight_registry.rs:104-117` (SK-062). Same observation.

