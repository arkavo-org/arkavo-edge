# SwarmKit live PR-review execution — design

- **Date:** 2026-06-18
- **Branch:** `feature/swarmkit-pr-review` (from `main`)
- **Status:** Design — awaiting review before implementation plan
- **Supersedes / folds in:** draft PR #626 (`github-ops-kit` manifest), draft PR #625 (`run_eval` / `arkavo-eval`)
- **Related:** issues #574/#573 (SwarmKit), #608–#610 (SwarmKit), #636 + `docs/a2a/realignment-scope.md` (A2A realignment — interacts, see Structural Debt)

## Goal

Let a swarm of agents **monitor GitHub PRs and review them**, end-to-end, using the existing SwarmKit specialization model: roles + their MCP-tool grants are bundled and assigned to empty awaiting agents on the mesh, which then do the work. The worked example is the `github-ops-kit` (all four roles live): a `dispatcher` discovers PRs and routes them; a `pr_reviewer` posts a code review; a `pr_test_runner` runs the local-model eval gate and posts a check verdict; a `repo_maintainer` performs repo-scoped housekeeping.

All models are local ($0 budget). The feature ships as a complete, integrated capability on one branch off `main`.

## Background — what exists vs. what is missing

The SwarmKit *manifest model*, the *apply pipeline*, **mesh discovery**, **bundle crypto**, and the **Iroh data plane** all exist. What's missing is the thin composition that turns a manifest into running, PR-reviewing agents.

**Exists (production):**
- Manifest model — `arkavo-swarmkit` (`manifest.rs` `Manifest`, `role.rs` `RoleSpec` / `McpToolGrant`:262 / `AuthMode`). Parses + validates.
- `apply_kit` pipeline — `arkavo-orchestrator/src/swarmkit_apply.rs:217`: capability-match (`RoleCapabilityMatcher`:165) → per-role token scoping (`TokenVault::tokens_for_role`) → `build_bundle`:357 → wrap (`BundleEncryptor`:146) → ship (`BundleShipper`:157) → launch `SwarmFlight` → dispatch (`RoleTaskTransport`).
- `SwarmFlight` — `arkavo-swarmkit-runtime/src/flight.rs` (`launch`:193, `RoleRuntime`:111, `record_tool_outcome`:303).
- **Bundle crypto (handled)** — `arkavo-protocol/src/agent_specialization.rs` `wrap_bundle`:254 / `unwrap_bundle`:270: canonical JSON → TDF policy bound to recipient DID → encrypt/decrypt with dissemination pre-flight. Round-trip test `swarmkit_bundle_round_trip.rs`. No crypto to design.
- **Mesh discovery (handled)** — agents announce over mDNS (`arkavo-cli/src/commands/agent.rs:1918` `broadcast_agent_mdns_sync`, service `_a2a._tcp.local.`, props `agent_id`/`purpose`/`capabilities`/`mcp_tools`/`public_key`); `arkavo-mcp-mesh/src/lib.rs:906` `discover_and_register_agents` populates `MeshToolsState.agent_addresses`:71 + `AgentRegistry::register_agent`:961. Address resolution + JSON-RPC send already exist via `SendTaskTool`:432/469.
- **Iroh data plane (handled)** — `arkavo-tdf-iroh` (`lib.rs:42` exports `IrohNode`/`IrohTicket`/`IrohTransport`): `IrohTransport.stage_bytes`→ticket (`transport.rs:50`), `fetch_bytes` (`transport.rs:76`). An `iroh_node` is embedded in every agent (`a2a_server.rs:91`); `iroh_stage`/`iroh_fetch` MCP tools already exist.
- `agent.specialize` RPC — `arkavo-server/src/server/mod.rs:275/1017` → `handlers/specialization.rs:57` (decode bundle → `unwrap_bundle` → apply persona+tokens to `AgentMetadata`:196 → store role context).
- Conductor execution — `conductor.rs:68` `execute_with_conductor_and_learning` → `conductor_tool_loop.rs:43` `run_tool_loop` → `ToolLoopResult`:22 (`final_text`, telemetry). Runs an agent task to completion in-process and returns the result.
- In-process MCP tool registry — `arkavo-mcp-tools` (`registry.rs`): `git_*`, `github_*`, `gh_pr_review`:220, `gh_checks`, `test_run`:241, `code_review`:304; `test_*` auto-categorized "Testing":655. In-process `Tool` impls; `McpToolGrant.server` is a logical label.
- GitHub ops — `arkavo-github` (`poller.rs`/`org_polling.rs` poll **issues** on a 5-min cron; `operations.rs` `create_pr`/`list_prs`/`merge_pr`; `issue_ops.rs` `post_comment`; `app_auth.rs` GitHub App / PAT). #625 adds `create_check_run`/`update_check_run`.
- Agent process — `arkavo agent` CLI (`commands/agent.rs:~1160` `start_agent_server`); binds an A2A endpoint; announces over mDNS; continuous agent loop (`agent_loop.rs:81`).

**Missing / stubbed (the work):**
1. **PR monitoring** — poller is issues-only; no `get_pr` / `list_open_prs` / PR diff; no `github_pr_watch` tool.
2. **Concrete mesh `BundleShipper` + `RoleTaskTransport`** — both are trait-only (test stubs). Missing is an `IrohBundleShipper` that *composes* existing infra: stage the (already-encrypted) TDF bundle on the Iroh data plane → resolve the agent address via mesh discovery → send `agent.specialize` carrying the **iroh ticket** over A2A. Plus a `RoleTaskTransport` that dispatches each role's task. The `agent.specialize` handler must **fetch the bundle blob from the ticket** (`IrohTransport.fetch_bytes`) before `unwrap_bundle` (today it expects an inline base64 bundle).
3. **Grant→registry filtering** — a specialized agent currently sees ALL tools (`agent_loop.rs:118-143`); `AgentMetadata` drops `persona.mcp_tools`; least-privilege not enforced.
4. **Post-specialize role execution** — the agent loop is continuous-advisory; nothing makes a specialized agent *do its role* (poll, review, gate).
5. **Production caller of `apply_kit`** — tests only.
6. **The `run_eval` gate** — lives only on draft #625.

## Decisions (locked)

| # | Decision | Rationale |
|---|---|---|
| D1 | **Maximal scope** — all four roles execute live. | User intent. |
| D2 | **One branch off `main`**, folding #625 + #626 in. | No stacked PRs; complete integrated feature. |
| D3 | **PR poller is an MCP tool** (`github_pr_watch`) declared in the manifest, granted to `dispatcher`; bundled to an empty awaiting agent. No bespoke runner. | Reuses the specialization model; keeps GitHub specifics out of core (generic apply→specialize→execute does the work). |
| D4 | **Distributed A2A mesh on the *current* A2A stack** — each empty agent is its own `arkavo agent` process; orchestrator ships bundles + dispatches over the mesh. | User chose true mesh now, accepting realignment rework (see Structural Debt). |
| D5 | **`repo_maintainer` does live, repo-scoped writes.** | User intent; gated (see Cross-cutting). |
| D6 | **`run_eval` → `test_eval` tool in `arkavo-mcp-tools`** (Testing category), feature-gated for llama.cpp; eval *pipeline* stays the `arkavo-eval` lib it depends on. Drop #625's `arkavo-server` `eval-tool` wiring and `arkavo-cli` dep. | `arkavo-mcp-tools` *is* the testing MCP (already hosts `test_run`); agents reach it through the standard registry; preserves slim/musl/Windows no-C++ builds. |
| D7 | **Bundle crypto is already handled** — TDF `wrap_bundle`/`unwrap_bundle` (recipient-DID-bound, dissemination-checked, round-trip tested). Reference it; build nothing. | Existing, tested infra. |
| D8 | **Discovery is already handled** (mDNS + `arkavo-mcp-mesh`); **ship bundles over the Iroh data plane** (`IrohTransport.stage_bytes`→ticket; agent `fetch_bytes`), ticket carried on the A2A `agent.specialize` call. No file registry, no self-registration. | Use existing mesh + data plane; matches the channel rule (large payload → Iroh, ticket via A2A). |
| D9 | **Least-privilege enforced, not conventional** — each specialized agent's registry contains only its granted tools, checked at call time. | Security; realizes the manifest's per-role grants. |

## Architecture

### Conceptual model

```
N× `arkavo agent`  ── already announce over mDNS (agent_id, capabilities, mcp_tools, pubkey)
        │              already discoverable: arkavo-mcp-mesh → MeshToolsState.agent_addresses + AgentRegistry
        │
`arkavo swarm apply github-ops-kit --org <ORG>`   (production apply_kit caller)
        │   match roles → AgentRegistry.find_best_agent → wrap bundle (existing TDF, per-recipient DID)
        │   → IrohBundleShipper: stage TDF bundle on Iroh → ticket
        │   → A2A agent.specialize(ticket) → dispatch role task (RoleTaskTransport)
        ▼
agent: fetch bundle blob via ticket (IrohTransport.fetch_bytes) → unwrap_bundle (existing)
       → persona + grants + scoped tokens → GRANT-FILTERED registry → runs its role
        ├─ dispatcher    : github_pr_watch (poll ORG) → for each new/changed PR → handoff to spokes
        ├─ pr_reviewer   : git_diff, gh_pr_review        → posts review        (own grant)
        ├─ pr_test_runner: test_eval, gh_checks          → posts gate verdict  (own grant)
        └─ repo_maintainer: git_*, github_issue_*, github_pr_create → repo-scoped writes (own grant)
```

The review and verdict are posted **by the role agents through their own granted tools** — no central result-collection plumbing for the primary outcome. The transport returns only lifecycle/status.

### Roles & grants (github-ops-kit, from #626 + D3/D6)

| Role | Type/plane | Model | Granted tools (`auth`) |
|---|---|---|---|
| `dispatcher` | planner / coordination | Gemma-4 E2B | `github_pr_watch` (delegated), `github_ci_status` (delegated), `github_related_issues` (delegated) |
| `pr_reviewer` | critic / coordination | Qwen3 7B | `git_diff` (none), `git_log` (none), `gh_pr_review` (delegated), `github_ci_status` (delegated) |
| `pr_test_runner` | operator / coordination | Gemma-4 E2B | `test_eval` (none), `gh_checks` (delegated), `github_ci_status` (delegated) |
| `repo_maintainer` | maintainer / coordination | Qwen3 7B | repo-scoped `git_*`, `github_issue_list`/`_create`, `github_pr_create`, `github_org_repos` (delegated) |

Least-privilege boundaries (asserted by test): reviewer cannot `github_pr_create`; maintainer cannot `gh_pr_review`; only `pr_test_runner` has `test_eval`.

### Component map

| Crate | Change |
|---|---|
| `arkavo-eval` | **Fold #625** as the eval **pipeline lib** (contract/operator/embedder/verdict/baseline). Drop its own MCP `tool.rs` registration, the `arkavo-server` `eval-tool` wiring, and the `arkavo-cli` dep. |
| `arkavo-mcp-tools` | New `eval.rs` → `test_eval` tool wrapping `arkavo-eval`; register in `registry.rs` next to `test_run`; behind optional `eval` feature (llama.cpp). |
| `arkavo-github` | New PR ops: `get_pr`, `list_open_prs(state=open, sort=updated)`, PR diff/files; PR cursor/dedup state (head-SHA) parallel to issue state. Fold #625 `create_check_run`/`update_check_run`. |
| `arkavo-mcp-tools` (PR watch) | New `github_pr_watch` tool: new/changed PRs for an org since a cursor (head-SHA dedup). Cursor state + PR-fetch live in `arkavo-github`; the tool **adds an `arkavo-github` dependency** — confirm no dep cycle. Registered in `registry.rs`. |
| `arkavo-orchestrator` | `IrohBundleShipper` (impl `BundleShipper`): stage wrapped bundle on `IrohTransport` → resolve agent address via mesh/`AgentRegistry` → A2A `agent.specialize(ticket)`. `RoleTaskTransport` impl (dispatch role task + start signal) + role-completion/status return. Production `apply_kit` wiring. |
| `arkavo-server` | `agent.specialize` handler: accept an **iroh ticket**, `fetch_bytes` → `unwrap_bundle` (existing crypto). Persist `persona.mcp_tools` in `AgentMetadata` → build a **grant-filtered `ToolRegistry`** + call-time enforcement (`agent_loop.rs:118`, `conductor_tool_loop.rs`). **Post-specialize role execution** (run the role's task through the conductor loop). |
| `arkavo-cli` | `arkavo swarm apply <manifest> --org <ORG> [--once] [--interval <m>]` (prod `apply_kit` caller); no `arkavo-eval` dep. |
| `examples/github-ops-kit` | **Fold #626** manifest + `github_ops_kit_validates` test; add the `github_pr_watch` grant to `dispatcher`; reference `test_eval` for `pr_test_runner`. |

## Workstreams (build order, all on this branch)

- **WS-A — Fold-ins.** Vendor `arkavo-eval` (pipeline lib, drop CLI/server special wiring); add `test_eval` to `arkavo-mcp-tools` behind the `eval` feature; vendor the `github-ops-kit` manifest + validation test. Independent; lands first.
- **WS-B — GitHub PR capability.** `arkavo-github` PR ops + state cursor; `github_pr_watch` MCP tool + check-run fold-in. Independent of the mesh.
- **WS-C — Least-privilege specialization (security).** Persist grants in `AgentMetadata`; grant-filter the agent's registry; enforce at call time; post-specialize role execution. Before WS-E. Includes the negative security test.
- **WS-D — Mesh bundle shipping (Iroh data plane).** `IrohBundleShipper` (stage on Iroh + A2A `agent.specialize` ticket); `agent.specialize` handler fetches the blob by ticket; `RoleTaskTransport` impl + status return. Reuses existing discovery + crypto. Before WS-E.
- **WS-E — Integration.** `arkavo swarm apply` production caller; manifest grant wiring; end-to-end github-ops-kit run; `repo_maintainer` repo-scoping + governance.

## Cross-cutting concerns

- **Least-privilege (D9):** the specialized agent's registry is the filtered set; an ungranted tool call is rejected pre-execution. Dedicated negative test (reviewer attempting `github_pr_create` is denied).
- **`repo_maintainer` scoping (D5):** the orchestrator pins the role's tool context to the triggering org/repo; repo args are validated against it (not model-trusted); manifest egress allowlist (github.com only) + `proposal_governance` + `on_failure: escalate / max 1 retry`; actions recorded in the decision trace.
- **Error semantics:** infra failures vs. review regressions are kept distinct (manifest success-criteria + `arkavo-eval` `TypedStatus::InfraError`). Never post a misleading review/verdict on infra failure. PR head-SHA cursor prevents re-review of unchanged PRs; a new push re-triggers. Budget: $0 cost cap (local only), 1800s/120k per the manifest, enforced per role via `derive_arp_for_role`.
- **Structural debt — A2A realignment (D4):** WS-D's discovery (mDNS/mesh), data plane (Iroh), and bundle crypto (TDF wrap/unwrap) **align with** the realignment direction (`docs/a2a/realignment-scope.md`: iroh discovery WS6, TDF Parts, KAS split). The remaining current-stack dependency is the **`agent.specialize` JSON-RPC shape + A2A JSON-RPC send**, which DEC-4 replaces with a `SendMessage` + TDF `Part` over `arkavo-config-transport`. Those two seams (`IrohBundleShipper`'s A2A call, the `agent.specialize` handler) carry a `// STRUCTURAL DEBT (a2a-realignment DEC-4):` marker and are logged in the PR description for migration. The debt is narrow because we reuse the surviving infra.

## Bundle transport over the mesh (existing infra + the one new piece)

Discovery, crypto, and the data plane already exist; WS-D only composes them.

1. **Discover (exists):** `apply_kit` matches each role to an agent via `AgentRegistry::find_best_agent` (populated by mDNS discovery); address resolved via `MeshToolsState.agent_addresses`.
2. **Wrap (exists):** `build_bundle` → `wrap_bundle(bundle, recipient_did)` → TDF bytes (recipient-DID-bound, dissemination-checked).
3. **Stage on Iroh (new glue):** `IrohBundleShipper.ship(agent_did, tdf_bytes)` → `IrohTransport.stage_bytes(tdf_bytes)` → ticket. (Bundle is a sizeable blob → Iroh per the channel rule.)
4. **Signal over A2A (new glue):** send `agent.specialize { ticket }` to the resolved address (reusing the mesh's JSON-RPC send path).
5. **Fetch + unwrap (small handler change):** the `agent.specialize` handler `IrohTransport.fetch_bytes(ticket)` → `unwrap_bundle` (existing) → apply persona/grants/tokens.

No file registry, no new crypto, no self-registration.

## Testing

- **WS-A:** `arkavo-eval` pipeline + `test_eval` tool tests (folded #625); manifest validation (`github_ops_kit_validates`, folded #626); `test_eval` lands in "Testing"; `eval` feature off ⇒ tool absent, builds clean.
- **WS-B:** `github_pr_watch` — new PR detected, head-SHA change re-triggers, unchanged skipped (mocked GitHub API); check-run create/update (folded #625).
- **WS-C:** grant-filtered registry contains only granted tools; **negative test**: ungranted tool call denied; post-specialize execution runs a role task and returns its result.
- **WS-D:** `IrohBundleShipper` stages a bundle and a second node fetches + `unwrap_bundle`s it (extends the existing `swarmkit_bundle_round_trip` test across two iroh nodes); `agent.specialize` handler resolves a ticket end-to-end.
- **WS-E:** end-to-end — agents announced on the mesh → `swarm apply` → dispatcher polls a mock GitHub → reviewer posts via mocked `gh_pr_review`, gate via `gh_checks`, maintainer writes scoped to the triggering repo (asserted).
- Regression test accompanying every bug fix; pre-push security suite (`e2e_security_test.sh`, `security_vulnerabilities`, `mock_provider`, DLP/PII).

## Out of scope (YAGNI)

- A2A realignment migration of the `agent.specialize` seam (tracked as debt; do when DEC-4 lands).
- Webhooks (the `github_pr_watch` poll tool replaces them).
- General test execution beyond the model-regression gate (`test_run` already exists for cargo tests; `test_eval` is the model gate).
- A new discovery mechanism (mDNS/mesh already provides it).

## Risks & open questions

- **R1 — A2A rework cost (accepted, narrowed):** only the `agent.specialize` RPC seam is current-stack debt; discovery/data-plane/crypto survive the realignment. Mitigated by the `BundleShipper`/`RoleTaskTransport` trait seams + debt markers.
- **R2 — N agent processes on one host:** lifecycle (spawn/supervise/teardown) for the empty-agent pool. Confirm during planning whether `swarm apply` spawns the pool or assumes pre-started, mesh-announced `arkavo agent` processes.
- **R3 — delegated GitHub tokens:** `gh_pr_review`/`gh_checks` shell out to `gh`; `github_*` API tools use the `TokenVault`. Confirm the per-role token source is consistent for delegated grants.
- **R4 — `test_eval` baseline bootstrapping:** first run on a repo has no baseline (bootstraps, neutral verdict). Confirm desired first-run behavior for the gate.

## References

- Manifest/runtime: `arkavo-swarmkit/src/{manifest,role}.rs`, `arkavo-swarmkit-runtime/src/flight.rs`, `arkavo-orchestrator/src/swarmkit_apply.rs`.
- Specialize/exec: `arkavo-server/src/server/{mod.rs:275/1017, handlers/specialization.rs, conductor.rs:68, conductor_tool_loop.rs:43, agent_loop.rs:118}`.
- Crypto/discovery/data-plane (existing): `arkavo-protocol/src/agent_specialization.rs:254/270`, `arkavo-protocol/tests/swarmkit_bundle_round_trip.rs`, `arkavo-cli/src/commands/agent.rs:1918` (mDNS), `arkavo-mcp-mesh/src/lib.rs:906/71/432`, `arkavo-tdf-iroh/src/{lib.rs:42, transport.rs:50/76}`, `arkavo-server/src/server/a2a_server.rs:91`.
- Tools: `arkavo-mcp-tools/src/registry.rs` (`test_run`:241, `gh_pr_review`:220, Testing:655), `test_runner.rs`, `github_review.rs`, `github_checks.rs`.
- GitHub: `arkavo-github/src/{poller.rs, org_polling.rs, operations.rs, issue_ops.rs, app_auth.rs}`.
- Fold-ins: PR #625 (`arkavo-eval`), PR #626 (`github-ops-kit`).
- Realignment interaction: `docs/a2a/realignment-scope.md`, issue #636.
