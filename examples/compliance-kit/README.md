# Compliance Kit (3-agent regulated-domain SwarmKit)

Vertical-slice SwarmKit demonstrating a PII compliance workflow:
a document goes in; a redacted document plus an audit-ready
compliance report come out. Pipeline: pii_classifier →
policy_enforcer → auditor.

This kit exists primarily to demonstrate **per-role TDF
attribute-release policies**. Each role carries different attribute
sets (clearance, jurisdiction, audit_authority) so the orchestrator
can issue role-scoped TDF policies — exactly the §6.4 capability
the runtime ships.

## Roles

| id | role_type | model | purpose |
|---|---|---|---|
| `pii_classifier` | `pii_classifier` | qwen3 7B | Classify documents for PII per jurisdiction. |
| `policy_enforcer` | `policy_enforcer` | qwen3 7B | Apply jurisdiction-aware redaction or escalation. |
| `auditor` | `auditor` | qwen3 7B | Produce audit-ready compliance report. Critic for the kit's evaluation rubric (separate evaluating role — not self-evaluation laundering per spec §10.1). |

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
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```

## Skills

The three skills are inline-signed with `did:web:arkavo.com`. The
deterministic dev signing key (`[7u8; 32]`) is for reproducibility.

To regenerate signatures:

```bash
cargo run -p arkavo-swarmkit-runtime --example sign_compliance_skills
```

Then update the YAML's `signature` fields and recompute `kit.id`:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```

Set `kit.id` in the YAML to the computed value.

## Out of scope for this MVP

- Live PII classifier model — the kit specifies the workflow; the
  runtime doesn't ship a PII model.
- Multi-jurisdiction at once — this YAML is hardcoded to
  `jurisdiction/us-ca` on every role's TDF policy. Producers
  fork the YAML for other jurisdictions (eu, us-hipaa, etc.).
- A2A JSON-RPC delegation envelope — defined in spec §7.2 but not yet wired.
- `source: tdf-ref` skills — Phase 2 supports `inline` and `registry` only.
- Live KAS integration for the per-role TDF policies — the kit
  declares the attribute sets; orchestrator-side TDF wrapping
  happens via `arkavo_swarmkit_runtime::role_policy` (already
  wired per SK-053).
