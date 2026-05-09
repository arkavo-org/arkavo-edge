# r/arkavo Reddit post: SwarmKit launch

> **⚠️ DO NOT POST until the umbrella PR (arkavo-org/arkavo-edge#590) has merged to `main`.** The post body links to `https://github.com/arkavo-org/arkavo-edge/blob/main/...` paths. These resolve only after the launch lands on main. To verify before posting, run:
>
> ```bash
> for url in \
>   "https://github.com/arkavo-org/arkavo-edge/blob/main/docs/SWARMKIT.md" \
>   "https://github.com/arkavo-org/arkavo-edge/blob/main/docs/swarmkit-launch-audit-2026-05-08.md" \
>   "https://github.com/arkavo-org/arkavo-edge/blob/main/examples/campaign-kit/README.md" \
>   "https://github.com/arkavo-org/arkavo-edge/blob/main/examples/code-review-kit/README.md" \
>   "https://github.com/arkavo-org/arkavo-edge/blob/main/examples/vrm-production-kit/README.md" \
>   "https://github.com/arkavo-org/arkavo-edge/blob/main/examples/compliance-kit/README.md"; do
>   code=$(curl -sI -o /dev/null -w "%{http_code}" -L "$url")
>   echo "$code  $url"
> done
> ```
> Every URL must return 200. If any return 404, the PR hasn't fully merged yet — wait.

> **Suggested title** (pick one when posting):
> - "SwarmKit ships: per-role TDF policies, signed skills, four runnable kits"
> - "We just shipped SwarmKit — declarative multi-agent kits where roles enforce their own data boundary"

> **Post body** (copy from `---` to the end into Reddit's editor; Reddit accepts plain Markdown).

---

Each agent in your swarm sees only the data its role permits.

Most agent frameworks treat the swarm as one trust boundary. SwarmKit pushes the boundary inward: every role declares its own TDF Attribute Release Policy, and the orchestrator constructs role-scoped policies before any data reaches the role.

The compliance-kit demonstrates this concretely. Three roles share `clearance/restricted + jurisdiction/us-ca`, but only the auditor carries `audit_authority/true`:

```yaml
# pii_classifier
tdf_attribute_release_policy:
  attributes:
    - "https://attr.arkavo.com/role/pii_classifier"
    - "https://attr.arkavo.com/clearance/restricted"
    - "https://attr.arkavo.com/jurisdiction/us-ca"
  rule: "allOf"

# auditor (privilege differentiator)
tdf_attribute_release_policy:
  attributes:
    - "https://attr.arkavo.com/role/auditor"
    - "https://attr.arkavo.com/clearance/restricted"
    - "https://attr.arkavo.com/jurisdiction/us-ca"
    - "https://attr.arkavo.com/audit_authority/true"
  rule: "allOf"
```

The orchestrator's `role_policy()` function translates each role's manifest block into an OpenTDF `Policy`. The KAS enforces it at unwrap time. Even if the orchestrator (compromised or not) tries to hand the auditor's data to another role, the rewrap fails. The trust boundary moves inside the swarm.

## Four shipped kits

- **campaign-kit** (marketing): analyst → copy → critic
- **code-review-kit** (developer): reviewer → security_auditor → test_writer
- **vrm-production-kit** (creative): prompt_designer → vrm_assembler → validator
- **compliance-kit** (regulated): pii_classifier → policy_enforcer → auditor

Each kit ships with a signed YAML manifest, a six-section README, and a closeout integration test that asserts every role's skills resolve with `verified=true`.

## What works, what's deferred

| Capability | Status |
|---|---|
| Per-role TDF attribute-release policies | wired |
| Skill signature verification (ed25519 over BLAKE3 canonical) | wired |
| TDF envelope wrap/unwrap + KAS-gated decrypt | wired |
| Manifest parser + cross-block validation | wired |
| A2A JSON-RPC delegation envelope (spec §7.2) | aspirational |
| Specialist process spawning + inference | aspirational |
| Manifest-level signing (TDF assertions) | aspirational |

## Run it

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```

Or auto-launch via the gateway:

```bash
ARKAVO_SWARMKIT_PATH=examples/compliance-kit/compliance-kit.swarmkit.yaml arkavo
```

## And the role names are yours

The spec defines `role_type` as a free-form string. A conformance test (`SK-006`) proves orchestrators don't reject domain-specific values. The four kits each pick their own — `pii_classifier`, `vrm_assembler`, `code_reviewer`, `asset_analyst`. Domain-specific role types travel through the manifest, the runtime, the panel, and the audit trail unchanged. The recommended vocabulary (`scribe`, `historian`, `planner`, etc.) is exactly that — recommended, not required.

## Links

- Canonical guide: [docs/SWARMKIT.md](https://github.com/arkavo-org/arkavo-edge/blob/main/docs/SWARMKIT.md)
- v2 audit (87 invariant-level rows, what's wired, what's deferred): [docs/swarmkit-launch-audit-2026-05-08.md](https://github.com/arkavo-org/arkavo-edge/blob/main/docs/swarmkit-launch-audit-2026-05-08.md)
- Per-kit READMEs: [campaign-kit](https://github.com/arkavo-org/arkavo-edge/blob/main/examples/campaign-kit/README.md), [code-review-kit](https://github.com/arkavo-org/arkavo-edge/blob/main/examples/code-review-kit/README.md), [vrm-production-kit](https://github.com/arkavo-org/arkavo-edge/blob/main/examples/vrm-production-kit/README.md), [compliance-kit](https://github.com/arkavo-org/arkavo-edge/blob/main/examples/compliance-kit/README.md)

Roadmap is in `docs/SWARMKIT.md`; happy to dig into any of the deferred items in comments.
