# Code Review Kit (3-agent developer-domain SwarmKit)

Vertical-slice SwarmKit demonstrating a code-review workflow: a code diff
goes in; a structured review (correctness findings, security findings,
generated tests) comes out. Pipeline: reviewer → security_auditor →
test_writer. The reviewer doubles as the rubric critic.

## Roles

| id | role_type | model | purpose |
|---|---|---|---|
| `reviewer` | `code_reviewer` | qwen3 7B | Review diff for correctness, design, naming, style. Critic for the kit's evaluation rubric. |
| `security_auditor` | `security_auditor` | qwen3 7B | Scan diff for OWASP/CWE-class defects. |
| `test_writer` | `test_author` | gemma-4 9B | Write unit + negative-path tests for changed code paths. |

`role_type` is free-form per spec §4.3; `code_reviewer`, `security_auditor`,
and `test_author` are domain-specific values not in Appendix C.

## Topology

`pipeline` — reviewer → security_auditor → test_writer. No back-edges.
Reviewer hands off correctness + design findings to security_auditor;
both feed into test_writer's coverage targets.

## Constraints

- 5-minute wallclock budget, 60k token budget, $0.20 cost cap.
- All data classified `internal`; `network_egress: false` everywhere.
- `process` sandbox per role.

## Validate

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/code-review-kit/code-review-kit.swarmkit.yaml
```

Parses, validates cross-block invariants, computes the BLAKE3 `kit.id`.

## Skills

The three skills are inline-signed with `did:web:arkavo.com`. The
deterministic dev signing key (`[7u8; 32]`) is for reproducibility —
producers replace it with their own.

To regenerate signatures when content changes:

```bash
cargo run -p arkavo-swarmkit-runtime --example sign_code_review_skills
```

Edit the YAML's `signature` and `signed_by` fields with the output, then
recompute `kit.id`:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/code-review-kit/code-review-kit.swarmkit.yaml
```

Set `kit.id` in the YAML to the computed value.

## Out of scope for this MVP

- Actual code-review inference — this kit specifies the workflow; the
  runtime doesn't yet have a specialist process to run the models.
- A2A JSON-RPC delegation envelope — defined in spec §7.2 but not yet wired.
- `source: tdf-ref` skills — Phase 2 supports `inline` and `registry` only.
