# AGENTS.md → SwarmKit-Only Config Migration

**Date:** 2026-07-11  
**Status:** In progress (S0 locked, S1+S2 loader landed on branch)  
**Execution mode:** Local branch, targeted tests, checkpoints; **one final GitHub PR** (see [`agent-harness-local-workflow.md`](agent-harness-local-workflow.md))  
**Branch:** `feature/swarmkit-only-config`

## S0 decisions (locked)

| Question | Decision |
|----------|----------|
| Unsigned local kits | Allowed when `runtime.local_dev: true` (consumers honor this; production leaves unset/false) |
| Multi-agent examples | **One kit per mesh** (multi-role), not one kit per agent directory |
| Config RPC names | Keep `agent_config_*` method names for now; payload becomes kit YAML (rename later with A2A if needed) |
| Sequence vs harness | **SwarmKit-only config first**; harness must not add new AGENTS.md call sites |
| Root monorepo Agents.md | **Keep** as contributor coding guidelines (not runtime config) |

## Goal

Remove product support for **AGENTS.md as agent configuration** and make **SwarmKit YAML** (`.swarmkit.yaml`) the only supported format for:

- Agent identity (name, purpose / objective, mode)
- Model + inference provisioning
- MCP tool grants
- Budgets and isolation
- Skills / system instructions
- Multi-agent roles and handoffs
- Policy surfaces currently stuffed into AGENTS.md frontmatter (preflight, cloud spend, KAS)

After cutover:

| Input | Status |
|-------|--------|
| `*.swarmkit.yaml` / `ARKAVO_SWARMKIT_PATH` | **Only** config format |
| `.arkavo/AGENTS.md`, `./AGENTS.md` product configs | **Rejected** with migration message |
| `arkavo agent init` | Writes a **minimal single-role SwarmKit**, not AGENTS.md |
| Examples under `examples/**/AGENTS.md` | Converted to kits or single-role kits |
| Repo coding guidelines `AGENTS.md` / `Agents.md` at monorepo root | **Out of scope** (tooling for human/AI contributors, not runtime config) |

## Non-goals

- Deleting root `Agents.md` / `AGENTS.md` **coding-agent guidelines** used by Claude Code / Grok / CI agents (different artifact; rename only if product confuses users — optional follow-up).
- Implementing full A2A realignment.
- Changing SwarmKit cryptographic kit.id rules without need.
- Storing API keys in SwarmKit (keys stay env-only: `XAI_API_KEY`, etc.).

## Why this is hard

AGENTS.md is not one feature — it is a **config bus** with ~46 Rust files and dozens of example trees.

| Concern | Today (AGENTS.md) | SwarmKit today | Gap |
|---------|-------------------|----------------|-----|
| Multi-agent sections | `## name` blocks | `roles[]` | Map 1:1 |
| Purpose / system identity | `purpose:` | `objective.goal` + role `description` + skill `instructions` | Need single-role “identity skill” convention |
| Model hint | `model:` | `agent_provisioning.model` | Map family/size |
| Mode orchestrator/specialist | `mode:` | `plane` + handoffs + launch options | Explicit `runtime.mode` or derive |
| MCP servers | nested markdown list | `mcp_tools` grants (names) | Commands/URLs may need kit-level `mcp_servers` extension |
| Listen / mDNS | `listen:`, `mdns:` | Not in manifest | Kit-level `runtime.network` extension or keep CLI flags only |
| Preflight policies | YAML frontmatter | None | **Must add** kit- or role-level `preflight` |
| $ budget / cloud_policy | YAML `budget:` | Token/wallclock budget only | **Must add** spend plane fields |
| KAS | YAML `kas:` | TDF role policies exist; KAS enable/key_id partial | Map into kit constraints / role TDF |
| Human teaching parse | `parse_agents_md` lessons | Skills + ARP | Load lessons from skill text / kit objective |
| Config RPC get/update/validate/restore | AGENTS.md file CRUD | SwarmKit path CRUD | Rewrite handlers |
| Auto-generate on first run | Writes AGENTS.md | Should write minimal kit | Change default path |
| Claude Code agent blocks | Custom AGENTS sections | Not modeled | Kit skill or drop if unused |

## Target model

### Single agent (hello-world)

One file, one role — full SwarmKit schema, not a second mini-format:

```yaml
spec_version: "1.0.0"
kit:
  id: "blake3:…"          # computed at init / validate
  name: "hello-agent"
  version: "0.1.0"
  description: "Friendly single agent"
  authors: [{ did: "did:key:…", name: "local" }]
  created: "…"
  expires: "…"
  nonce: "…"

objective:
  goal: "Introduce yourself and answer basic questions"
  success_criteria: ["responds helpfully"]

roles:
  - id: "agent"
    role_type: "operator"
    description: "Primary agent"
    agent_provisioning:
      model:
        family: "ministral"
        size: "3B"
        backend: "llama.cpp"
      budget:
        max_inference_calls: 32
        max_total_tokens: 100000
      isolation:
        sandbox: "process"
        network_egress: false
    skills:
      - id: "skill:identity"
        version: "0.1.0"
        source: "inline"
        payload:
          name: "identity"
          description: "System identity and operating instructions"
          instructions: |
            You are a friendly agent that introduces itself and answers basic questions.
          resources: []
        # signature optional for local-only kits if validator allows unsigned local
    mcp_tools: []   # or explicit grants
    handoffs: []
```

### Multi-agent mesh

Replace N× `AGENTS.md` directories with **one kit** (or one kit per mesh) declaring all roles — same as campaign-kit / code-review-kit.

### Runtime discovery order (final)

```text
1. ARKAVO_SWARMKIT_PATH if set
2. .arkavo/*.swarmkit.yaml (prefer single file; error if multiple without flag)
3. ./*.swarmkit.yaml
4. Else: zero-config single-role default kit in memory OR refuse with “run arkavo kit init”
```

**Never** open `AGENTS.md` for runtime config.

If `AGENTS.md` exists, log once:

```text
AGENTS.md is no longer supported. Convert with:
  arkavo kit migrate-from-agents-md --in AGENTS.md --out agent.swarmkit.yaml
```

---

## Schema extensions (before cutover)

Add to `arkavo-swarmkit` (validated, optional fields so existing kits stay valid):

### Kit-level `runtime` (optional)

```yaml
runtime:
  mode: orchestrator | specialist   # default orchestrator for single-role kits
  listen: "0.0.0.0:0"               # optional; CLI may override
  mdns: true
  cloud_policy: local_only | ask_before_cloud | cloud_within_cap
  max_cost_per_session: 1.0         # dollars; maps to BudgetYamlConfig
  max_cost_per_day: 5.0
  kas:
    enabled: false
    key_id: null
    algorithm: null
  preflight:
    policies: [...]                 # same shape as today’s PreflightConfig
  mcp_servers:                      # if grants need process spawn metadata
    - name: filesystem
      command: …
      args: []
```

### Role-level (optional)

- Keep existing `agent_provisioning`, `mcp_tools`, skills.
- Allow role `preflight` override if needed later (v1 kit-level only is enough).

### Validation rules

- Existing SK-* still pass for kits without `runtime`.
- If `runtime.preflight` present → same policy validation as router today.
- `cloud_policy` enum validation.
- Single-role kits valid without handoffs.

Specs: extend `swarmkit.spec.yaml` (SK-1xx) for new fields; retire AGENTS-specific scenarios in `agent.spec.yaml`, `cli.spec.yaml`, `chat-session` refs.

---

## Capability mapping (implementation lookup)

| AGENTS.md field | SwarmKit target | Loader code today to retarget |
|-----------------|-----------------|-------------------------------|
| `## name` | `roles[].id` / `kit.name` | `parse_agents_config` |
| `purpose` | skill `instructions` + `objective.goal` | a2a_server purpose / system prompt |
| `model` | `agent_provisioning.model` | model_hint resolution |
| `mode` | `runtime.mode` | `AgentMode` |
| `mdns` / `listen` | `runtime.*` or CLI flags | agent start |
| `mcp_servers` | `runtime.mcp_servers` + `mcp_tools` grants | MCP registry build |
| frontmatter `preflight` | `runtime.preflight` | `load_agent_config` / moderator |
| frontmatter `budget` | `runtime.cloud_*` + role `budget` | spend plane |
| frontmatter `kas` | `runtime.kas` + role TDF | tdf_audit |
| free-text lessons | skills / objective | `parse_agents_md` / learning bus |

---

## Phase plan (local phases, not intermediate GitHub PRs)

Aligned with local-only workflow: **no CI until final PR**.

### Phase S0 — Inventory freeze + design lock

**Work:**

- Land this doc; list every load site (grep `AGENTS.md` / `parse_agents_config` / `load_agent_config`).
- Decide: unsigned local kits allowed for `arkavo kit init`? (**Recommend yes**, with `local_dev: true` or skip signature verify when `ARKAVO_SWARMKIT_INSECURE_LOCAL=1` / path under `.arkavo/`.)
- Decide: keep root monorepo `Agents.md` coding guidelines (yes).

**Tier T:** none (docs only).

### Phase S1 — Schema + parse `runtime` block

**Work:**

- Extend `arkavo-swarmkit` types + validate.
- Unit tests for optional `runtime` parse/reject bad enums.
- Spec scenarios SK-1xx.

**Tier T:**

```bash
cargo fmt -- --check
cargo nextest run -p arkavo-swarmkit
```

### Phase S2 — Runtime config loader (SwarmKit → AgentRuntimeConfig)

**Work:**

- New module (prefer `arkavo-swarmkit` or thin `arkavo-config` path):  
  `load_swarmkit_config(path) -> AgentRuntimeConfig`  
  containing everything router/server need today from `AgentConfig` (protocol) + router `AgentConfig` (preflight/budget/kas).
- Single type becomes the **only** config DTO; dual AGENTS parsers not called by new path.
- Feature-flag or env: `ARKAVO_CONFIG_SOURCE=swarmkit` default once ready; temporary dual-read for internal testing only (must not ship dual-read in final).

**Tier T:**

```bash
cargo nextest run -p arkavo-swarmkit -p arkavo-swarmkit-runtime
cargo nextest run -p arkavo-router --lib  # once wired
```

### Phase S3 — Wire server + router + CLI to SwarmKit loader

**Work (replace call sites, do not leave AGENTS paths):**

| Area | Files (indicative) |
|------|---------------------|
| Server start | `a2a_server.rs`, `startup.rs`, `local_engine.rs`, `spend_plane.rs` |
| Config RPC | `handlers/config.rs`, `config_helpers.rs` — get/update/validate/restore **kit YAML** |
| Router | `preflight/config.rs` — load from kit `runtime.preflight` |
| CLI | `commands/agent.rs`, `lib.rs` API keys (env only), `tool_integration.rs`, `model.rs` |
| Learning | `learning_bus_synthesis.rs` — lessons from skill instructions |
| AG-UI | `gateway.rs`, `security_handler.rs` |

**Behavior:**

- Prefer `ARKAVO_SWARMKIT_PATH` / `.arkavo/*.swarmkit.yaml`.
- If only AGENTS.md present → **error** with migrate command (no silent fallback in final behavior; mid-phase may warn).

**Tier T:**

```bash
cargo nextest run -p arkavo-server --lib
cargo nextest run -p arkavo-protocol
cargo nextest run -p arkavo-cli --lib
cargo nextest run -p arkavo-router --no-default-features --features llm-remote,gemini
```

### Phase S4 — CLI: `kit init` / `kit migrate` / remove AGENTS generation

**Work:**

- `arkavo kit init <name>` → writes `.arkavo/<name>.swarmkit.yaml` (minimal single-role), prints validate command.
- `arkavo kit migrate-from-agents-md --in … --out …` → best-effort conversion.
- `arkavo agent init` → deprecate alias to `kit init` for one release, then remove.
- Stop auto-creating AGENTS.md on first run (`tests/default_agent_run.rs` rewrite).
- Remove `agents_md.prompt.md` generation path or retarget to swarmkit prompt.

**Tier T:**

```bash
cargo nextest run -p arkavo-cli --lib
cargo test -p arkavo-cli --test …   # whatever covers init
```

### Phase S5 — Examples + docs migration

**Work:**

- Convert `examples/**/AGENTS.md` → `*.swarmkit.yaml` (or one kit per multi-agent example).
- Update `examples/README.md`, `docs/CAPABILITIES.md`, `README.md` quickstart.
- Update `docs/SWARMKIT.md` “single-agent is a one-role kit”.
- Secure-agent preflight examples become kit `runtime.preflight`.
- Delete obsolete AGENTS.md under examples once converted.

**Tier T:** validate kits:

```bash
cargo run -p arkavo-swarmkit --example validate_kit -- examples/01-hello-world/*.swarmkit.yaml
# … per converted kit
```

### Phase S6 — Delete AGENTS.md product code paths

**Work:**

- Remove or gut `parse_agents_config` product entrypoints (protocol + cli duplicates).
- Remove AGENTS search in `load_agent_config` / `load_policies_from_agents_md` (replace with kit loaders only).
- Remove `parse_agents_md` teaching path or reimplement on skills.
- Delete tests that only exist for AGENTS.md CRUD; replace with SwarmKit config tests.
- Specs: agent.spec.yaml parse scenarios → SwarmKit; cli “API keys from AGENTS.md” → env + kit.
- Grep gate: `rg 'AGENTS\.md' crates --glob '*.rs'` should only hit **migration error strings** and maybe comments in CHANGELOG/docs.

**Tier T + Checkpoint SC1:**

```bash
cargo fmt -- --check
cargo build -q
# harness-style package clippy + tests for touched crates
rg 'parse_agents_config|load_agent_config_from_agents_md|load_policies_from_agents_md' crates --glob '*.rs'
# expect empty (or only #[cfg(test)] migration tests if kept temporarily)
```

### Phase S7 — Final gate + single PR

Same as local workflow **Tier F**:

- Full harness-related + swarmkit + server + cli + protocol tests
- Security scripts if config surfaces touch DLP paths
- Version bump
- Push `feature/swarmkit-only-config` once
- One PR: title e.g. `SwarmKit-only agent configuration`

---

## Checkpoint schedule

| Checkpoint | After | Proves |
|------------|-------|--------|
| **SC0** | S0–S1 | Schema accepts `runtime`; old kits still validate |
| **SC1** | S2–S3 | Server boots with kit only; AGENTS.md ignored/errors |
| **SC2** | S4–S5 | Init + examples work without AGENTS.md |
| **SC3** | S6 | Grep clean; dead parsers gone |
| **Final** | S7 | Tier F + open PR |

---

## Migration tool (S4 detail)

`arkavo kit migrate-from-agents-md`:

1. Parse legacy markdown (reuse existing parser **only inside this command** until S6 deletes it).
2. Emit one-role kit per `##` agent, or multi-role kit if multiple sections in one file.
3. Map `purpose` → identity skill instructions + objective.goal.
4. Map frontmatter YAML → `runtime.*`.
5. Print kit.id after `validate` / recompute blake3.
6. Exit non-zero if unmapped fields remain (list them).

Keep migrate command **one release** after cutover if needed; do not keep runtime AGENTS load.

---

## Test strategy

| Layer | Action |
|-------|--------|
| Unit | SwarmKit `runtime` parse; loader maps model/budget/preflight |
| Integration | Server start with only kit path; config RPC update kit content |
| Negative | Presence of AGENTS.md does not change behavior (except warning) |
| Example | Each converted example validates + smoke `arkavo` where feasible |
| Regression | Replace `tests/agent_config_test.rs`, `default_agent_run.rs`, terminal `agents_md_test` |

Do **not** rely on GitHub CI until Final.

---

## Risk register

| Risk | Mitigation |
|------|------------|
| SwarmKit validation too strict for local hello-world (signatures, kit.id) | Local init generates valid id; optional unsigned local policy |
| Examples break overnight demos | Convert high-traffic examples first (01-hello-world, secure-agent, mesh) |
| Dual config types (protocol AgentConfig vs router AgentConfig) | One `AgentRuntimeConfig` DTO in S2 |
| API keys in old AGENTS.md | Never put keys in kits; document env vars only |
| Specialize / A2A bundle still mentions AGENTS | Update `agent_specialization` + realignment docs to kit bundle |
| Confusion with monorepo Agents.md | Docs explicitly split “contributor guidelines” vs “runtime kit” |

---

## Ordering vs agent-harness plan

| Option | When |
|--------|------|
| **A. Config migration first** | Cleaner harness: ToolLoop reads SwarmKit grants/budget only |
| **B. Harness first** | Faster chat depth; more AGENTS.md call sites to re-touch later |
| **C. Parallel branches** | High merge cost |

**Recommend A** if SwarmKit-only is a hard product requirement: do S0–S3 before deep harness chat cutover, so PR2/PR5 of the harness plan never re-bind to AGENTS.md.

If harness MVP is already in flight, do S1–S2 in parallel and **do not** add new AGENTS.md call sites in harness work.

---

## Acceptance criteria (ship)

1. Fresh clone: no AGENTS.md required to run a single agent; kit init or default kit path works.
2. `ARKAVO_SWARMKIT_PATH=examples/…/….swarmkit.yaml` boots roles as today.
3. Grep in `crates/**/*.rs`: no product load of `AGENTS.md` (migration error string OK).
4. All shipped examples use SwarmKit only.
5. Specs updated; AGENTS-as-config scenarios removed or marked superseded.
6. One GitHub PR; CI green after Final Gate.

## Open questions (resolve in S0)

1. **Unsigned local kits:** allow for `.arkavo/` only, or require signatures always?  
   *Recommendation:* allow unsigned when kit has no `provenance.signatures` and path is local-dev; document clearly.
2. **Multi-file mesh (one AGENTS.md per agent dir):** one kit vs kit-per-agent?  
   *Recommendation:* one kit per mesh example; single-process multi-role.
3. **Config RPC surface:** keep method names `agent_config_*` with kit body, or rename to `kit_config_*`?  
   *Recommendation:* rename in same PR if A2A realignment not blocking; else keep names, change payload to YAML kit.
4. **Root monorepo Agents.md:** leave as contributor guidelines?  
   *Recommendation:* leave; optional rename to `CONTRIBUTING.agents.md` later to reduce confusion.

---

## Related docs

- [`docs/SWARMKIT.md`](SWARMKIT.md)
- [`specs/arkavo-edge/swarmkit.spec.yaml`](../specs/arkavo-edge/swarmkit.spec.yaml)
- [`docs/agent-harness-pr-plan.md`](agent-harness-pr-plan.md)
- [`docs/agent-harness-local-workflow.md`](agent-harness-local-workflow.md)
- [`docs/a2a/realignment-scope.md`](a2a/realignment-scope.md) (config off-wire → kit transport)
