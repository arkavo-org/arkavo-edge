# SwarmKit

Each agent in your swarm sees only the data its role permits.

Most agent frameworks treat the swarm as one trust boundary. SwarmKit pushes the boundary inward: every role declares its own TDF Attribute Release Policy, and the orchestrator constructs role-scoped policies before any data reaches the role.

## What it is

A SwarmKit is a YAML manifest that declares roles, per-role agent provisioning, per-role TDF attribute-release policies, an evaluation rubric, and completion rules. The runtime takes the manifest, builds one Agent Runtime Policy (ARP) per role, and isolates per-role state (DecisionTrace, PolicyCache) at flight launch. Skills are inline-signed (ed25519 over BLAKE3 of canonical content); the runtime verifies signatures eagerly when `LaunchOptions::resolver_config` is set.

Three subsystems:

- **Manifest** (`arkavo-swarmkit` crate) — parser + cross-block validator. `kit.id` is BLAKE3 of the JCS-canonical manifest with `kit.id` and `provenance.signatures` stripped — content-addressed.
- **Runtime** (`arkavo-swarmkit-runtime` crate) — `SwarmFlight::launch` builds per-role ARP runtimes, isolates DecisionTrace + PolicyCache, optionally resolves and verifies skill signatures via the `PublicKeyResolver` trait. Production resolver: `DidWebPublicKeyResolver` (`did:web` only in this MVP, sync via `ureq`).
- **Gateway integration** (`arkavo-agui` crate) — `ARKAVO_SWARMKIT_PATH` env var auto-launches a kit when the AG-UI gateway boots, which happens under `arkavo ui`; the AG-UI panel surfaces every role under `flight:<flight_id>:<role_id>`.

## A single agent is a one-role kit

There is no separate single-agent config format — running one agent means authoring a kit with exactly one entry in `roles`. `arkavo agent -c <kit> [-p <port>]` runs it directly; multi-role kits add `-n <role-id>` to pick which role a given process runs. `arkavo kit init <name>` scaffolds this minimal shape; see `examples/01-hello-world/hello-agent.swarmkit.yaml` for a complete single-role kit.

A role's `agent_provisioning.model` picks the model `arkavo agent` runs it on, local or cloud. A local edge model is a family/size pair (`family: ministral`, `size: 3B`); any other model the router knows, cloud models included, is its router id as the family with no size (`family: gpt-6-astra`, `family: kimi-k2.5`). Naming a cloud model counts as consent to use it under the kit's `runtime.cloud_policy`. `arkavo agent` refuses to start a kit whose role names a model the router does not know, and `arkavo kit validate` fails on one.

Four example kits in the repository's `examples/` directory, each with its own README:

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

The runtime's `role_policy()` (in `crates/arkavo-swarmkit-runtime/src/tdf.rs`) translates each role's `tdf_attribute_release_policy` block into an OpenTDF `Policy` via `arkavo_tdf::PolicyBuilder`. It is exercised by the runtime crate's tests; the `arkavo ui` and `arkavo agent` launch paths do not call it yet (see [What a kit launch does today](#what-a-kit-launch-does-today)). The rest of this section describes the design those policies implement. The Key Access Service (KAS) enforces these policies at unwrap time. Even if the orchestrator (compromised or not) tries to hand the auditor's data to another role, the rewrap fails — the trust boundary is no longer "outside the swarm vs. inside" but "between any two roles within the swarm."

This is structurally different from agent-level access control, which gates access *to the orchestrator*. SwarmKit gates access *per role*. A compromised single role cannot exfiltrate other roles' data because the policy is enforced cryptographically before the data ever reaches it.

Ecosystem ties:

- **TØR-G** ([torg-decision spec](https://github.com/arkavo-org/specifications/tree/main/torg-decision)) for decision provenance — every per-role outcome lands in a DecisionTrace that can feed a TØR-G stream.
- **OpenTDF** ([opentdf.io](https://opentdf.io)) — the policy enforcement substrate. SwarmKit constructs OpenTDF `Policy` objects per role; KAS enforces them.
- **DIF/ToIP DID resolution** — currently `did:web` only via `DidWebPublicKeyResolver`; trait-extensible to `did:key`/`did:plc` (one new file per method).

## How to run it

The example kits live in the repository's `examples/` directory. Homebrew, the `.pkg`, the `.deb`, and the release archives install only the `arkavo` binary, so clone the repository to get them:

```bash
git clone https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge
```

With only the binary installed, `arkavo kit init <name>` is the starting point; see [Author your own kit](#author-your-own-kit).

### What a kit launch does today

Launching a kit into the gateway (`ARKAVO_SWARMKIT_PATH=<kit> arkavo ui`) parses the manifest and builds, for each role:

- an Agent Runtime Policy (ARP) runtime, made of a policy cache, an adaptation engine, and a decision trace, derived from the role's `agent_provisioning` and the kit's `constraints.global_budget`;
- the role's resolved skills;
- an entry in the AG-UI ARP panel under `flight:<flight_id>:<role_id>`.

It does not:

- spawn a process or load a model for any role;
- execute `handoffs` or pass work from one role to the next;
- apply the `evaluation` rubric or the `completion` rules;
- take the kit's `inputs` or write its `deliverables`;
- build the roles' TDF attribute-release policies;
- enforce skill signatures, unless `ARKAVO_SWARMKIT_VERIFY=required` is set. By default signatures are parsed and not enforced, because the example kits are signed with a local development key.

A launched kit is a set of per-role policy and audit records that you can inspect in the panel. To run the roles, start each one as an agent, as shown below.

### Validate

```bash
arkavo kit validate examples/compliance-kit/compliance-kit.swarmkit.yaml
```

`arkavo kit validate` accepts several paths. It fails on an expired kit, on a `kit.id` that does not match the manifest, and on a role that names a model the router does not know. It also lists the manifest controls that are declared but not enforced on the `arkavo agent -c` path.

### Load a kit into the gateway

```bash
ARKAVO_SWARMKIT_PATH=examples/compliance-kit/compliance-kit.swarmkit.yaml arkavo ui
```

Roles surface in the AG-UI ARP panel. This applies to builds with the web renderer (Homebrew, the `.deb`, and the release archives). The macOS `.pkg` build opens a native window and does not start the gateway, so it does not load the kit.

Bare `arkavo` does not start the gateway either. With `ARKAVO_SWARMKIT_PATH` set it runs the kit's first role as a single agent.

### Run the roles as agents

Each role runs as its own process. Start one per role, each on its own port:

```bash
arkavo agent -c examples/compliance-kit/compliance-kit.swarmkit.yaml -n pii_classifier -p 8341
arkavo agent -c examples/compliance-kit/compliance-kit.swarmkit.yaml -n policy_enforcer -p 8342
arkavo agent -c examples/compliance-kit/compliance-kit.swarmkit.yaml -n auditor -p 8343
```

`-n` takes a role `id` from the manifest; without it the first role runs. Each process takes the role's id, model, and skill instructions, and the kit-level `runtime` block (`listen`, `mdns`, `mode`, `mcp_servers`, `preflight`, `cloud_policy`). The role's `isolation`, network egress, budget, and `mcp_tools` grant fields are not enforced on this path.

Send a running role work with `arkavo chat --agent-id <role-id>` or `arkavo task --agent-id <role-id> '<task>'`.

Agents listen on `127.0.0.1` unless the kit sets `runtime.listen`, so roles on different devices need it:

```yaml
runtime:
  listen: "0.0.0.0:0"   # all interfaces; -p still sets the port
```

The agent's RPC endpoint does not authenticate callers yet, so bind it to other interfaces only on a trusted network.

### Author your own kit

With the binary alone, generate a single-role kit and build on it:

```bash
arkavo kit init my-kit
# writes .arkavo/my-kit.swarmkit.yaml; edit it to add roles, models, and skills
arkavo kit validate .arkavo/my-kit.swarmkit.yaml
arkavo agent -c .arkavo/my-kit.swarmkit.yaml
```

From a git checkout you can start from an example kit instead. The copied manifest keeps its old file name, so rename it:

```bash
cp -r examples/campaign-kit examples/my-kit
mv examples/my-kit/campaign-kit.swarmkit.yaml examples/my-kit/my-kit.swarmkit.yaml
# edit examples/my-kit/my-kit.swarmkit.yaml: kit name, created/expires dates, role IDs, descriptions, attributes
arkavo kit validate examples/my-kit/my-kit.swarmkit.yaml
```

`kit.id` is a hash of the manifest, so any edit makes the declared id stale. Set `kit.id: ""` while you edit; `arkavo kit validate` then prints the computed id. Paste it into `kit.id` when the manifest is final.

Skill signatures cover the skill content, so an edited skill needs a new signature. The signing helpers are source-only and each one signs the skills of one example kit with a development key: `cargo run -p arkavo-swarmkit-runtime --example sign_campaign_skills` prints the payload, `signature`, and `signed_by` values for the campaign-kit skills. To sign different content, change the skill text in the helper to match your manifest.

Per-kit READMEs in `examples/<kit>/README.md` document each kit's role decomposition, evaluation rubric, and TDF attribute-release sets.

### From source

The same validation is available without an installed binary:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```

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
| Per-role TDF attribute-release policies | wired in the runtime library (§6.4 / SK-053..054); not called by a launch path |
| Skill signature verification (ed25519 over BLAKE3 canonical) | wired (SK-090..092); enforced at gateway launch only with `ARKAVO_SWARMKIT_VERIFY=required` |
| TDF envelope wrap/unwrap + KAS-gated decrypt | wired (SK-050..062) |
| `ARKAVO_SWARMKIT_PATH` auto-launch | wired under `arkavo ui` (SK-022, SK-061..062) |
| AG-UI panel: live SwarmFlight roles | wired (SK-020..033) |
| Operator stop control (`requestStopFlight`) | wired (SK-040) |
| A2A JSON-RPC delegation envelope (§7.2) | aspirational |
| Specialist process spawning + inference | aspirational; start each role with `arkavo agent -c <kit> -n <role-id>` |
| `handoffs`, evaluation rubric, completion rules, `inputs`, `deliverables` | parsed and validated; not executed |
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
