# SwarmKit

Each agent in your swarm sees only the data its role permits.

Most agent frameworks treat the swarm as one trust boundary. SwarmKit pushes the boundary inward: every role declares its own TDF Attribute Release Policy, and the orchestrator constructs role-scoped policies before any data reaches the role.

## What it is

A SwarmKit is a YAML manifest that declares roles, per-role agent provisioning, per-role TDF attribute-release policies, an evaluation rubric, and completion rules. The runtime takes the manifest, builds one Agent Runtime Policy (ARP) per role, and isolates per-role state (DecisionTrace, PolicyCache) at flight launch. Skills are inline-signed (ed25519 over BLAKE3 of canonical content); the runtime verifies signatures eagerly when `LaunchOptions::resolver_config` is set.

Three subsystems:

- **Manifest** (`arkavo-swarmkit` crate) — parser + cross-block validator. `kit.id` is BLAKE3 of the JCS-canonical manifest with `kit.id` and `provenance.signatures` stripped — content-addressed.
- **Runtime** (`arkavo-swarmkit-runtime` crate) — `SwarmFlight::launch` builds per-role ARP runtimes, isolates DecisionTrace + PolicyCache, optionally resolves and verifies skill signatures via the `PublicKeyResolver` trait. Production resolver: `DidWebPublicKeyResolver` (`did:web` only in this MVP, sync via `ureq`).
- **Gateway integration** (`arkavo-agui` crate) — `ARKAVO_SWARMKIT_PATH` env var auto-launches a kit at gateway boot; the AG-UI panel surfaces every role under `flight:<flight_id>:<role_id>`.

Four shipped kits in `examples/`, each with its own README:

| Kit | Domain | Roles |
|---|---|---|
| [campaign-kit](../examples/campaign-kit/README.md) | Marketing | analyst → copy → critic |
| [code-review-kit](../examples/code-review-kit/README.md) | Developer | reviewer → security_auditor → test_writer |
| [vrm-production-kit](../examples/vrm-production-kit/README.md) | Creative | prompt_designer → vrm_assembler → validator |
| [compliance-kit](../examples/compliance-kit/README.md) | Regulated | pii_classifier → policy_enforcer → auditor |

## Why role-boundary trust matters

The compliance-kit demonstrates per-role TDF policy enforcement concretely. Three roles share `clearance/restricted + jurisdiction/us-ca`, but only the `auditor` carries `audit_authority/true`:

| role | attributes |
|---|---|
| `pii_classifier` | `role/pii_classifier`, `clearance/restricted`, `jurisdiction/us-ca` |
| `policy_enforcer` | `role/policy_enforcer`, `clearance/restricted`, `jurisdiction/us-ca` |
| `auditor` | `role/auditor`, `clearance/restricted`, `jurisdiction/us-ca`, **`audit_authority/true`** |

The orchestrator's `role_policy()` (in `crates/arkavo-swarmkit-runtime/src/tdf.rs:186-245`) translates each role's `tdf_attribute_release_policy` block into an OpenTDF `Policy` via `arkavo_tdf::PolicyBuilder`. The Key Access Service (KAS) enforces these policies at unwrap time. Even if the orchestrator (compromised or not) tries to hand the auditor's data to another role, the rewrap fails — the trust boundary is no longer "outside the swarm vs. inside" but "between any two roles within the swarm."

This is structurally different from agent-level access control, which gates access *to the orchestrator*. SwarmKit gates access *per role*. A compromised single role cannot exfiltrate other roles' data because the policy is enforced cryptographically before the data ever reaches it.

Ecosystem ties:

- **TØR-G** ([torg-decision spec](https://github.com/arkavo-org/specifications/tree/main/torg-decision)) for decision provenance — every per-role outcome lands in a DecisionTrace that can feed a TØR-G stream.
- **OpenTDF** ([opentdf.io](https://opentdf.io)) — the policy enforcement substrate. SwarmKit constructs OpenTDF `Policy` objects per role; KAS enforces them.
- **DIF/ToIP DID resolution** — currently `did:web` only via `DidWebPublicKeyResolver`; trait-extensible to `did:key`/`did:plc` (one new file per method).

## How to run it

Validate any of the four kits:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```

Launch a kit at gateway boot — roles surface in the AG-UI ARP panel:

```bash
ARKAVO_SWARMKIT_PATH=examples/compliance-kit/compliance-kit.swarmkit.yaml arkavo
```

Author your own kit:

```bash
cp -r examples/campaign-kit examples/my-kit
# edit examples/my-kit/my-kit.swarmkit.yaml — change role IDs, descriptions, attributes
cargo run -p arkavo-swarmkit-runtime --example sign_campaign_skills
# paste the printed signatures into the YAML's signature/signed_by fields, then re-validate
# (validate_kit will print the recomputed BLAKE3 kit.id; paste it into kit.id)
```

Per-kit READMEs in `examples/<kit>/README.md` document each kit's role decomposition, evaluation rubric, and TDF attribute-release sets.

## The role names are yours

The SwarmKit spec defines `role_type` as a free-form string. A conformance test (`SK-006`) proves orchestrators MUST NOT reject manifests with domain-specific values. The four shipped kits each pick their own:

- campaign-kit: `asset_analyst`, `platform_copy`, `critic`
- code-review-kit: `code_reviewer`, `security_auditor`, `test_author`
- vrm-production-kit: `prompt_designer`, `vrm_assembler`, `vrm_validator`
- compliance-kit: `pii_classifier`, `policy_enforcer`, `auditor`

This isn't a label system. Domain-specific role types travel through the manifest, the runtime, the panel, and the audit trail unchanged. The recommended vocabulary in spec Appendix C (`scribe`, `historian`, `planner`, `critic`, `operator`, `specialist`) is exactly that — recommended, not required.

## Status: what's wired, what's deferred

| Capability | Status |
|---|---|
| Manifest parser + cross-block validation | wired (SK-001..006) |
| Per-role ARP runtime construction at launch | wired (SK-010..015) |
| Per-role TDF attribute-release policies | wired (§6.4 / SK-053..054) |
| Skill signature verification (ed25519 over BLAKE3 canonical) | wired (SK-090..092) |
| TDF envelope wrap/unwrap + KAS-gated decrypt | wired (SK-050..062) |
| `ARKAVO_SWARMKIT_PATH` auto-launch | wired (SK-022, SK-061..062) |
| AG-UI panel: live SwarmFlight roles | wired (SK-020..033) |
| Operator stop control (`requestStopFlight`) | wired (SK-040) |
| A2A JSON-RPC delegation envelope (§7.2) | aspirational |
| Specialist process spawning + inference | aspirational |
| Manifest-level signing helper (TDF assertions) | aspirational |
| `source: tdf-ref` skills | roadmap |
| `did:key` / `did:plc` resolution | roadmap |

The full audit lives at [`swarmkit-launch-audit-2026-05-08.md`](swarmkit-launch-audit-2026-05-08.md) — 87 invariant-level rows, evidence cells with `file:line` references, four spec-gap categories. Diff against [`swarmkit-launch-audit-2026-05-07.md`](swarmkit-launch-audit-2026-05-07.md) is the evidence Phase 2 closed the only two ship-blocker rows in v1.

## Specification + roadmap

The SwarmKit specification is published at [`github.com/arkavo-org/specifications/tree/main/swarmkit`](https://github.com/arkavo-org/specifications/tree/main/swarmkit). Two drafts are live:

- [`swarmkit-spec-draft-00`](https://github.com/arkavo-org/specifications/blob/main/swarmkit/swarmkit-spec-draft-00.md) — the original spec defining manifest schema (§4), `agent_provisioning` (§5), TDF encryption envelope (§6), orchestrator decryption + delegation flow (§7), skills + MCP tool distribution (§8), versioning + identity (§9), security considerations (§10), and conformance criteria (§11).
- [`swarmkit-spec-draft-01`](https://github.com/arkavo-org/specifications/blob/main/swarmkit/swarmkit-spec-draft-01.md) — additive draft. Bumps `spec_version` to `1.1.0`. Promotes Phase 2's invented Skill resolver protocol (SkillContent JSON schema, Ed25519/BLAKE3 signing, registry cache layout) to normative status. Closes the v2 audit's spec-language-vs-runtime gaps (§4.6, §9.1, §1.2 promoted to MUST). Adds the audit-authority privilege-differentiation threat model entry (§10.1) and the Domain-Specific Examples appendix.

Changelog: [`CHANGELOG.md`](https://github.com/arkavo-org/specifications/blob/main/swarmkit/CHANGELOG.md). Future drafts will land here.

RFC AE-2026-004 (forthcoming) targets the formal IETF-style draft for community review.

Phase 5 spec proposals (sourced from the v2 audit's "Skill resolver gaps" subsection):

- SkillContent JSON schema (Phase 2 invented `{ name, description, instructions, resources }` — needs §8.1 sub-section).
- ed25519 over BLAKE3 of JCS-canonical bytes signing algorithm (Phase 2 invented; needs normative §8.1 paragraph).
- Registry cache layout: `<blake3-hex>.skill.json` (and reserved `.sig.json` sidecar).
- Audit-authority attribute pattern (the compliance-kit's `audit_authority/true` privilege differentiator).

## Links

- Spec: [`swarmkit-spec-draft-00`](https://github.com/arkavo-org/specifications/blob/main/swarmkit/swarmkit-spec-draft-00.md), [`draft-01`](https://github.com/arkavo-org/specifications/blob/main/swarmkit/swarmkit-spec-draft-01.md), [CHANGELOG](https://github.com/arkavo-org/specifications/blob/main/swarmkit/CHANGELOG.md)
- v2 audit: [`swarmkit-launch-audit-2026-05-08.md`](swarmkit-launch-audit-2026-05-08.md)
- v1 audit (Phase 1 snapshot): [`swarmkit-launch-audit-2026-05-07.md`](swarmkit-launch-audit-2026-05-07.md)
- Per-kit READMEs: [campaign-kit](../examples/campaign-kit/README.md), [code-review-kit](../examples/code-review-kit/README.md), [vrm-production-kit](../examples/vrm-production-kit/README.md), [compliance-kit](../examples/compliance-kit/README.md)
- Companion specs: [Agent Runtime Policy (ARP)](https://github.com/arkavo-org/specifications/tree/main/agent-runtime-policy), [TØR-G (decision provenance)](https://github.com/arkavo-org/specifications/tree/main/torg-decision)
- OpenTDF (policy enforcement substrate): [opentdf.io](https://opentdf.io)
- Issues: file at the [arkavo-edge issue tracker](https://github.com/arkavo-org/arkavo-edge/issues).
