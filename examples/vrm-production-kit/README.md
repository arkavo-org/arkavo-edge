# VRM Production Kit (3-agent creative-domain SwarmKit)

Vertical-slice SwarmKit demonstrating a VRM 1.0 avatar production
workflow: a creator brief goes in; a VRM-spec-compliant avatar
specification (JSON metadata) comes out. Pipeline:
prompt_designer → vrm_assembler → validator.

The kit *specifies* the workflow; producing actual `.vrm` binary
files is out of scope for this MVP — that would require a model
plus a glTF/VRM binary emitter (Phase 5 candidate).

## Roles

| id | role_type | model | purpose |
|---|---|---|---|
| `prompt_designer` | `prompt_designer` | gemma-4 9B | Produce avatar specification (visual brief, persona, constraints) from a creator brief. |
| `vrm_assembler` | `vrm_assembler` | gemma-4 9B | Emit VRM 1.0 metadata JSON (skeleton, blendshapes, metadata fields). |
| `validator` | `vrm_validator` | qwen3 7B | Validate VRM spec compliance + prompt fidelity. Critic for the kit's evaluation rubric. |

## Topology

`pipeline` — prompt_designer → vrm_assembler → validator. Validator
double-duties as the rubric critic; the assembler does not iterate
on validator feedback in this MVP (`completion.on_failure: abort`).

## Constraints

- 8-minute wallclock budget, 80k token budget, $0.40 cost cap. Creative
  work is allowed slightly more time than the other kits.
- All data classified `internal`; `network_egress: false` everywhere.
- `process` sandbox per role.
- Spec compliance threshold is 0.95 — VRM is a strict standard.

## Validate

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/vrm-production-kit/vrm-production-kit.swarmkit.yaml
```

## Skills

The three skills are inline-signed with `did:web:arkavo.com`. The
deterministic dev signing key (`[7u8; 32]`) is for reproducibility.

To regenerate signatures when content changes:

```bash
cargo run -p arkavo-swarmkit-runtime --example sign_vrm_production_skills
```

Then update the YAML's `signature` fields and recompute `kit.id`:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/vrm-production-kit/vrm-production-kit.swarmkit.yaml
```

Set `kit.id` in the YAML to the computed value.

## Skill signature verification at gateway boot

Skills in this kit are signed (ed25519 over BLAKE3 of the JCS-canonical
`SkillContent`) using the local dev key emitted by
`sign_vrm_production_skills`. That signer DID is not a published `did:web` document, so
the production resolver (`DidWebPublicKeyResolver`) cannot fetch its
public key.

To avoid dead-ending the first boot, gateway auto-launch via
`ARKAVO_SWARMKIT_PATH` defaults to `VerifyMode::Optional`: signatures
are parsed and surfaced on `ResolvedSkill`, but a missing or
unresolvable signer does not fail the launch. A `tracing::warn!` line
fires on boot so the trade-off is visible.

To enforce verification:

1. Replace the dev signer with one whose DID is a resolvable `did:web`
   document (re-run `sign_vrm_production_skills` with your own key, then paste the new
   `signature` and `signed_by` into the YAML).
2. Set `ARKAVO_SWARMKIT_VERIFY=required` in the gateway environment.

Code paths that bypass the gateway (custom `LaunchOptions`) keep
`VerifyMode::Required` as the explicit default.

## Out of scope for this MVP

- Actual `.vrm` binary emission — the kit specifies the workflow; the
  runtime doesn't yet have the specialist process or binary emitter.
- A2A JSON-RPC delegation envelope — defined in spec §7.2 but not yet wired.
- `source: tdf-ref` skills — Phase 2 supports `inline` and `registry` only.
- Style-reference TDF-blob input — declared in `inputs` for forward
  compatibility but not consumed by any role in this MVP.
