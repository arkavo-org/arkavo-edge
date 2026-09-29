# Compliance Kit (3-agent regulated-domain SwarmKit)

Vertical-slice SwarmKit demonstrating a PII compliance workflow:
a document goes in; a redacted document plus an audit-ready
compliance report come out. Pipeline: pii_classifier →
policy_enforcer → auditor.

This kit exists primarily to demonstrate **per-role TDF
attribute-release policies**. Each role carries different attribute
sets (clearance, jurisdiction, audit_authority) so the orchestrator
can issue role-scoped TDF policies (spec §6.4). The runtime library
builds those policies with `role_policy()`; the `arkavo ui` and
`arkavo agent` launch paths do not call it yet.

## Roles

| id | role_type | model | purpose |
|---|---|---|---|
| `pii_classifier` | `pii_classifier` | qwen 9B | Classify documents for PII per jurisdiction. |
| `policy_enforcer` | `policy_enforcer` | qwen 9B | Apply jurisdiction-aware redaction or escalation. |
| `auditor` | `auditor` | qwen 9B | Produce audit-ready compliance report. Critic for the kit's evaluation rubric (separate evaluating role — not self-evaluation laundering per spec §10.1). |

## Topology

`pipeline` — pii_classifier → policy_enforcer → auditor. The auditor
sees output from both upstream roles; this is the spec-aligned pattern
for evaluation, not single-role self-evaluation.

## TDF attribute-release per role

| role | attributes |
|---|---|
| `pii_classifier` | `role/pii_classifier`, `clearance/restricted`, `jurisdiction/us-ca` |
| `policy_enforcer` | `role/policy_enforcer`, `clearance/restricted`, `jurisdiction/us-ca` |
| `auditor` | `role/auditor`, `clearance/restricted`, `jurisdiction/us-ca`, `audit_authority/true` |

The auditor's `audit_authority/true` attribute is the privilege
differentiator — only the auditor sees data tagged with that
attribute. This is the SwarmKit-level expression of "role-scoped
TDF policy" (§6.4).

## Constraints

- 4-minute wallclock budget, 50k token budget, $0.15 cost cap.
- All data classified `restricted` (stricter than the other kits' `internal`).
- `network_egress: false` everywhere.
- `process` sandbox per role.
- PII recall threshold 0.95 — false negatives are the worst-case outcome.

## Validate

```bash
arkavo kit validate examples/compliance-kit/compliance-kit.swarmkit.yaml
```

Loads the manifest, validates cross-block invariants, and checks that the declared `kit.id` matches the BLAKE3 hash of the canonical-form manifest. It fails on an expired kit and on a role that names a model the router does not know.

From a source checkout without an installed binary, the equivalent is:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```

## Run

Load the kit into the web UI gateway to see one entry per role in the AG-UI ARP panel:

```bash
ARKAVO_SWARMKIT_PATH=examples/compliance-kit/compliance-kit.swarmkit.yaml arkavo ui
```

Loading builds per-role policy, trace, and panel entries. It does not start the roles or run the pipeline between them; see [What a kit launch does today](../../docs/SWARMKIT.md#what-a-kit-launch-does-today).

Start each role as its own agent process:

```bash
arkavo agent -c examples/compliance-kit/compliance-kit.swarmkit.yaml -n pii_classifier -p 8341
arkavo agent -c examples/compliance-kit/compliance-kit.swarmkit.yaml -n policy_enforcer -p 8342
arkavo agent -c examples/compliance-kit/compliance-kit.swarmkit.yaml -n auditor -p 8343
```

## Skills

The three skills are inline-signed with `did:web:arkavo.com`. The
deterministic dev signing key (`[7u8; 32]`) is for reproducibility.

To regenerate signatures:

```bash
cargo run -p arkavo-swarmkit-runtime --example sign_compliance_skills
```

Then update the YAML's `signature` fields, set `kit.id` to `""`, and recompute it:

```bash
arkavo kit validate examples/compliance-kit/compliance-kit.swarmkit.yaml
```

Set `kit.id` in the YAML to the printed value.

## Skill signature verification at gateway boot

Skills in this kit are signed (ed25519 over BLAKE3 of the JCS-canonical
`SkillContent`) using the local dev key emitted by
`sign_compliance_skills`. That signer DID is not a published `did:web` document, so
the production resolver (`DidWebPublicKeyResolver`) cannot fetch its
public key.

To avoid dead-ending the first boot, gateway auto-launch via
`ARKAVO_SWARMKIT_PATH` defaults to `VerifyMode::Optional`: signatures
are parsed and surfaced on `ResolvedSkill`, but a missing or
unresolvable signer does not fail the launch. A `tracing::warn!` line
fires on boot so the trade-off is visible.

To enforce verification:

1. Replace the dev signer with one whose DID is a resolvable `did:web`
   document (re-run `sign_compliance_skills` with your own key, then paste the new
   `signature` and `signed_by` into the YAML).
2. Set `ARKAVO_SWARMKIT_VERIFY=required` in the gateway environment.

Code paths that bypass the gateway (custom `LaunchOptions`) keep
`VerifyMode::Required` as the explicit default.

## Out of scope for this MVP

- Live PII classifier model — the kit specifies the workflow; the
  runtime doesn't ship a PII model.
- Multi-jurisdiction at once — this YAML is hardcoded to
  `jurisdiction/us-ca` on every role's TDF policy. Producers
  fork the YAML for other jurisdictions (eu, us-hipaa, etc.).
- A2A JSON-RPC delegation envelope — defined in spec §7.2 but not yet wired.
- `source: tdf-ref` skills — Phase 2 supports `inline` and `registry` only.
- Live KAS integration for the per-role TDF policies — the kit
  declares the attribute sets, and
  `arkavo_swarmkit_runtime::role_policy` turns them into OpenTDF
  policies (SK-053), but launching the kit does not build or enforce
  them.
