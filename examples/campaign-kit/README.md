# Campaign Kit (3-agent MVP)

Vertical-slice SwarmKit for [arkavo-org/arkavo-edge#573](https://github.com/arkavo-org/arkavo-edge/issues/573). Demonstrates the contract a Creator-side caller would send to Edge to generate a campaign output bundle.

## Roles

| id | role_type | model | purpose |
|---|---|---|---|
| `analyst` | `asset_analyst` | gemma-4 9B | Summarize source asset, extract selling points |
| `copy` | `platform_copy` | gemma-4 9B | Write platform-specific copy from selling points |
| `critic` | `critic` | qwen3 7B | Score against the rubric, flag unsupported claims |

`role_type` is free-form per spec §4.3; the values above are domain-specific to campaign workflows and will not appear in the recommended vocabulary (Appendix C of `swarmkit-spec-draft-00.md`).

## Topology

`pipeline` — analyst → copy → critic. No back-edges. The critic does not feed corrections back into copy in this MVP; on rubric failure the SwarmFlight aborts (see `completion.on_failure: abort`, `max_retries: 1`).

## Constraints

- 5-minute wallclock budget, 60k token budget, $0.25 cost cap.
- All data classified `internal`; `network_egress: false` everywhere.
- `process` sandbox per role.

## Validate

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/campaign-kit/campaign-kit.swarmkit.yaml
```

The example binary parses, validates cross-block invariants (per spec §4.6, §5.1, §10.1), and computes the BLAKE3 `kit.id` from the canonical-form manifest. The declared `kit.id` in the YAML is the result of that computation; any edit to the manifest will require recomputing the id (or temporarily setting it to `""` to skip the §9.1 hash check during authoring).

## Skills

The three skills are inline-signed with `did:web:arkavo.com`. The
deterministic dev signing key (`[7u8; 32]`) is for reproducibility —
producers replace it with their own.

To regenerate signatures when content changes:

```bash
cargo run -p arkavo-swarmkit-runtime --example sign_campaign_skills
```

Edit the YAML's `signature` and `signed_by` fields with the output, then
recompute `kit.id`:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/campaign-kit/campaign-kit.swarmkit.yaml
```

Set `kit.id` in the YAML to the computed value.

## Skill signature verification at gateway boot

Skills in this kit are signed (ed25519 over BLAKE3 of the JCS-canonical
`SkillContent`) using the local dev key emitted by
`sign_campaign_skills`. That signer DID is not a published `did:web`
document, so the production resolver (`DidWebPublicKeyResolver`) cannot
fetch its public key.

To avoid dead-ending the first boot, gateway auto-launch via
`ARKAVO_SWARMKIT_PATH` defaults to `VerifyMode::Optional`: signatures
are parsed and surfaced on `ResolvedSkill`, but a missing or
unresolvable signer does not fail the launch. A `tracing::warn!` line
fires on boot so the trade-off is visible.

To enforce verification:

1. Replace the dev signer with one whose DID is a resolvable `did:web`
   document (re-run `sign_campaign_skills` with your own key, then paste
   the new `signature` and `signed_by` into the YAML).
2. Set `ARKAVO_SWARMKIT_VERIFY=required` in the gateway environment.

Code paths that bypass the gateway (custom `LaunchOptions`) keep
`VerifyMode::Required` as the explicit default.

## Out of scope for this MVP

- A2A JSON-RPC delegation envelope — defined in spec §7.2 but not yet wired.
- Creator UI / approval screen — lives in the Arkavo Creator codebase.
- `source: tdf-ref` skills — Phase 2 supports `inline` and `registry` only.
