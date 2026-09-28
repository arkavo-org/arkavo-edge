# Agent Harness Unification — PR Plan

**Date:** 2026-07-11  
**Status:** Ready for execution  
**Branch:** one long-lived `feature/agent-harness` (see local workflow)  
**Source review:** Grok 4.5 agent-harness review (chat session + specs/ + server/CLI loops)

**Execution mode:** Do **not** open intermediate GitHub PRs or use CI for each plan PR. Implement phases locally with targeted tests and checkpoint regressions; open **one final PR**. Workflow: [`docs/agent-harness-local-workflow.md`](agent-harness-local-workflow.md).

## Goal

Make Arkavo Edge a best-in-class **coding agent harness** by:

1. Unifying three divergent runtimes (chat, CLI tool loop, server conductor) onto one tool-loop core.
2. Giving the **default interactive path** multi-step depth, quality gates, and durable context.
3. Specifying and testing the control loop (not only component crates).

This plan is incremental: each PR is independently reviewable and mergeable; later PRs deepen behavior without forcing a big-bang rewrite.

## Non-goals (this plan)

- A2A protocol realignment cutover (`docs/a2a/realignment-scope.md`) — only add harness gates so dual-path survives realignment.
- New LLM providers or model training.
- Replacing SwarmKit / HRM multi-agent mesh (those stay; coding harness becomes the default single-user path).
- Full OTEL productization beyond a minimal step transcript (PR 9).

## Current state (problem)

| Path | Entry | Max tool depth | Quality gate | Context |
|------|--------|----------------|--------------|---------|
| Chat | `chat_session` → `route_chat` | **1** execute + synthesize | No | Last **8** msgs, drop |
| CLI | `tool_integration::process_with_tools` | ~10 / 20 hard | Yes + critic | Caller-owned |
| Server | `agent_loop` → `conductor_tool_loop` / parallel | **4** seq / **2** plan rounds | Yes | Distill + ConversationWindow |

Best ingredients (three-plane router, progressive tool attach, ToolMemory, quality_gate, `arkavo-context`) exist but are path-local.

## Target architecture

```text
                    ┌─────────────────────────────────────┐
  User / A2A / CLI  │  Frontends (unchanged UX contracts)  │
                    │  chat_session · tool_integration ·   │
                    │  agent_loop / conductor              │
                    └─────────────────┬───────────────────┘
                                      │
                                      ▼
                    ┌─────────────────────────────────────┐
                    │  arkavo-agent-runtime (new crate)    │
                    │  ToolLoop + StopPolicy + StepEvent   │
                    │  ContextPolicy · MemoryPolicy ·      │
                    │  PermissionPolicy (plugins)          │
                    └─────────────────┬───────────────────┘
                                      │
              ┌───────────────────────┼───────────────────────┐
              ▼                       ▼                       ▼
         arkavo-router          arkavo-mcp-tools         arkavo-context
         (route, QG, planes)    (registry, execute)      (compact, enrich)
```

**Invariant:** Frontends own transport, auth, and product surface. The runtime owns **how many steps**, **when to stop**, **how context is compacted**, and **what events are recorded**.

## Key decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Shared loop location | New crate `arkavo-agent-runtime` | Avoids `protocol` ↔ `server` ↔ `cli` cycles; thin deps on router + tools + context traits |
| Migration strategy | Extract → dual-run chat → cut over CLI → cut over server | Chat is weakest UX and highest user impact; server is riskiest |
| Default coding depth | Budget-based (inferences + wall time + failed verifies), soft default 25 steps, hard cap 50 | Hardcoded 4 is too shallow; unbounded is unsafe |
| Compaction | Semantic distill via existing small-model path + must-preserve set | Cold drop of last-N loses goals/failures |
| Plan state | First-class `TaskPlan` in runtime, not only ToolMemory strings | Survives compaction; drives stop/verify |
| Permissions | Product modes (ReadOnly / Ask / Workspace / Full) layered on SwarmKit grants | Grants alone are specialize-only |
| Specs first for loop | Formal ATL/UAL/CAW specs land with or before code PRs that claim behavior | Specs today under-specify the harness |

## Dependency graph

```text
PR0  Specs hygiene + ATL/UAL/CAW skeletons
 │
 ▼
PR1  arkavo-agent-runtime crate (ToolLoop core, no frontend cutover)
 │
 ├──────────────┬──────────────┐
 ▼              ▼              ▼
PR2 Chat       PR3 Context    PR4 Quality on chat
 depth         compact API     (can parallel after PR1)
 │              │              │
 └──────┬───────┴──────┬───────┘
        ▼              ▼
       PR5 Plan/todo + verify outer loop
        │
        ├──────────────┬──────────────┐
        ▼              ▼              ▼
       PR6 Memory     PR7 Stuck +    PR8 Permission
       auto-retrieve  progress       modes + sandbox
        │              │              │
        └──────────────┴──────┬───────┘
                              ▼
                       PR9 Transcript + cost timeline
                              │
                              ▼
                       PR10 CLI + server cutover (delete dual paths)
                              │
                              ▼
                       PR11 Subagent explore/implement/verify (optional track)
```

PRs 2–4 can be a Graphite stack on top of PR1. PRs 6–8 can stack after PR5 or partially parallel if interfaces are stable.

---

## PR Plan

### PR0 — Harness specs and ID hygiene

**Title:** Agent harness behavior specs and scenario ID fixes  
**Size:** S  
**Depends on:** nothing  
**Branch:** `feature/harness-specs`

**Changes:**

- Add `specs/arkavo-edge/agent-tool-loop.spec.yaml` (ATL-001..007):
  - Max iterations / budget stop
  - Tool result size condense
  - Mid-loop compaction preserves last N + goal
  - Budget exhaustion without provider call
  - Granted-tools reject
  - Negative reward → planning profile
  - Schema/parse failure does not kill session
- Add `specs/arkavo-edge/unified-agent-loop.spec.yaml` (UAL-001..008):
  - IncomingMessage reaches tool-capable cycle
  - History across cycles
  - CycleReceipt dispositions
  - HumanOverride priority
  - Duplicate prompt skip
  - Dead-man clear_history
  - Adaptive interval on timeouts
  - Specialist path without event channel unchanged
- Add `specs/arkavo-edge/coding-agent-workflow.spec.yaml` (CAW-001..005):
  - search → read → edit → test green
  - test fail → re-edit ≤ N
  - workspace sandbox for destructive cmds
  - stop on max retries / budget, tree consistent
  - progressive tool disclosure for small models
- Fix `task-orchestration.spec.yaml` ID collision with `orchestrator.spec.yaml` (rename to TASK-*).
- Update `specs/arkavo-edge/index.yaml` and note in `specs/FUTURE.md` that component inventory ≠ harness completeness.
- Prefer **path-only** `refs:` (no line numbers) for new scenarios.

**Files:**

- Create: `specs/arkavo-edge/agent-tool-loop.spec.yaml`
- Create: `specs/arkavo-edge/unified-agent-loop.spec.yaml`
- Create: `specs/arkavo-edge/coding-agent-workflow.spec.yaml`
- Edit: `specs/arkavo-edge/task-orchestration.spec.yaml`
- Edit: `specs/arkavo-edge/index.yaml`
- Edit: `specs/FUTURE.md` (short note, not full rewrite)

**Acceptance:**

- Specs validate against `specs/schema.json`.
- No duplicate scenario IDs among ORCH/TASK/ATL/UAL/CAW.
- Scenarios reference real modules (may be `wip: true` until later PRs implement).

**Tests:** Schema validation only (no new runtime tests yet).

---

### PR1 — Extract shared ToolLoop runtime crate

**Title:** Shared agent ToolLoop runtime crate  
**Size:** L  
**Depends on:** PR0 (specs can be WIP)  
**Branch:** `feature/harness-tool-loop-core`

**Changes:**

- New crate `crates/arkavo-agent-runtime/` with:
  - `ToolLoopConfig` — max_steps, soft/hard caps, timeouts, model hints, quality flags
  - `StopReason` — Done | MaxSteps | BudgetExhausted | Stuck | Cancelled | VerifyFailed | Error
  - `StepEvent` — model, tokens, tools, latency, quality notes (in-memory for now)
  - `run_tool_loop(...)` ported from the **best of**:
    - `conductor_tool_loop.rs` (condense, compaction, grant check, reward replan)
    - `tool_integration.rs` (loop detectors, progressive discovery caps)
  - Pure helpers unit-tested: `tool_call_permitted`, result condense tiers, stop evaluation
- Server and CLI still call **old** functions; runtime is dual-implemented or thin-wrapper initially.
  - Preferred: move body into runtime; server re-exports/wraps to avoid behavior change.
- Workspace `Cargo.toml` member + feature flags minimal (`std` + existing deps: router, mcp-tools, budget).

**Files:**

- Create: `crates/arkavo-agent-runtime/Cargo.toml`
- Create: `crates/arkavo-agent-runtime/src/lib.rs`
- Create: `crates/arkavo-agent-runtime/src/loop.rs`
- Create: `crates/arkavo-agent-runtime/src/config.rs`
- Create: `crates/arkavo-agent-runtime/src/stop.rs`
- Create: `crates/arkavo-agent-runtime/src/events.rs`
- Create: `crates/arkavo-agent-runtime/src/condense.rs` (from conductor tiers)
- Edit: `Cargo.toml` (workspace members)
- Edit: `crates/arkavo-server/src/server/conductor_tool_loop.rs` (delegate to runtime)
- Edit: `specs/arkavo-edge/agent-tool-loop.spec.yaml` (clear `wip` for ATL units with tests)

**Acceptance:**

- No intentional behavior change on server path (golden: same max steps default 4 until PR2/5).
- Unit tests for ATL-001/002/004/005 style pure logic.
- `cargo test -p arkavo-agent-runtime`
- `cargo test -p arkavo-server --lib conductor_tool_loop` still green
- File size: keep modules under ~400 lines impl (Agents.md)

**Tests:**

- `tool_call_permitted` grant matrix
- Condense tier boundaries (40/60/80% / 3000 char)
- Stop when max_steps reached / budget exhausted
- Regression: existing conductor_tool_loop tests pass via delegate

---

### PR2 — Chat path multi-step ToolLoop

**Title:** Multi-step tool loop for interactive chat  
**Size:** L  
**Depends on:** PR1  
**Branch:** `feature/harness-chat-depth`

**Changes:**

- Replace single-shot tool batch + tool-less synthesis in `chat_session` with `arkavo-agent-runtime::run_tool_loop`.
- Defaults for interactive chat:
  - soft max_steps = 25, hard = 50
  - honor existing compute/budget if present
  - stream StepEvents as MessageDelta metadata where possible
- Tool results use **tool/assistant roles** consistent with Gemma/Llama templates (not stuffed solely as user).
- Keep MessageDelta protocol compatible (tool_call / tool_result / text).
- `CHAT_WINDOW_SIZE = 8` remains until PR3; loop depth is independent of window size.

**Files:**

- Edit: `crates/arkavo-protocol/src/chat_session.rs`
- Edit: `crates/arkavo-protocol/Cargo.toml` (dep on agent-runtime)
- Edit: `crates/arkavo-cli` / local engine wiring if chat constructs registry there
- Edit: `specs/arkavo-edge/chat-session.spec.yaml` (CHAT-009 expanded; multi-step)
- Edit: `specs/arkavo-edge/coding-agent-workflow.spec.yaml` (CAW-001 partial)

**Acceptance:**

- Prompt requiring ≥3 sequential tools (e.g. list → read → summarize) completes without human re-prompt.
- Single-turn no-tool chat unchanged in latency characteristics (no forced multi-round).
- Back-pressure / 100-delta limit still holds.
- No regression on CHAT-001..008, CHAT-010..012.

**Tests:**

- Unit/integration: multi-step tool mock registry (3 calls) ends with Done
- Max steps stop surfaces clear terminal message
- Streaming still delivers tool_call deltas in order

---

### PR3 — Semantic context compaction

**Title:** Semantic compaction with must-preserve set  
**Size:** M–L  
**Depends on:** PR1 (runtime hooks); can land parallel to PR2 if chat still uses window  
**Branch:** `feature/harness-context-compact`

**Changes:**

- Define `ContextPolicy` / `MustPreserve` in agent-runtime:
  - system constraints
  - active goal / plan summary
  - last verify/failure output
  - recent tool errors
  - correlation/task ids
- Wire `arkavo-context` compression / small-model distill (pattern from `conductor_tool_loop`) into:
  - runtime mid-loop compaction
  - chat conversation window replacement for cold drop
  - optional: ConversationWindow in server uses same policy
- Fix or quarantine zero embeddings: either compute real embeddings for conversation store **or** store without claiming semantic search (prefer real embeddings when memory feature on).
- Document single token-estimator strategy for budget gates (Llama tokenizer when available; chars/4 fallback) — one trait in runtime, used by chat + server.

**Files:**

- Edit: `crates/arkavo-agent-runtime/src/context_policy.rs` (new)
- Edit: `crates/arkavo-context/src/*` (public API for must-preserve compact if needed)
- Edit: `crates/arkavo-protocol/src/chat_session.rs` (drop fixed-8 cold drop)
- Edit: `crates/arkavo-server/src/server/conversation_window.rs` (optional summarize on trim)
- Edit: `crates/arkavo-session/src/conversation.rs` (embeddings fix)
- Edit: `specs/arkavo-edge/context.spec.yaml` (fix CTX-008 if possible; add fidelity scenarios)
- Edit: ATL-003

**Acceptance:**

- Over-budget conversation retains goal + last failure after compact.
- No silent all-zero embedding path for new conversation messages when embeddings enabled.
- CTX fidelity scenarios green or marked implemented.

**Tests:**

- Compaction preserves MustPreserve fields (unit)
- Chat multi-turn > window size still recalls earlier goal (integration with mock distill)

---

### PR4 — Quality gate and critic on chat path

**Title:** Quality gate on interactive chat tool routes  
**Size:** M  
**Depends on:** PR1, PR2  
**Branch:** `feature/harness-chat-quality`

**Changes:**

- Chat tool iterations use `route_with_tools_hinted` / quality_gate path instead of thin `route_chat` for tool-bearing turns.
- Enable MissingToolUse retry + schema validation on chat.
- Optional critic on high-risk tools (write_file, shell_exec, git push) — config flag default on for write/shell.
- Collapse detector surfaces upgrade offer metadata without auto-spend (existing three-plane rules).

**Files:**

- Edit: `crates/arkavo-protocol/src/chat_session.rs` or runtime quality hooks
- Edit: `crates/arkavo-router/src/quality_gate.rs` (only if chat needs a lighter profile)
- Edit: `crates/arkavo-agent-runtime` quality integration
- Edit: CHAT / ROUTER / CRIT specs as needed

**Acceptance:**

- Invalid tool schema triggers retry/feedback, not silent fail-closed session death.
- Write/shell can be blocked or flagged by critic when enabled.
- No silent cloud spend from collapse (ROUTER-021 still holds).

**Tests:**

- Mock bad tool JSON → replan/retry ≤ 3
- Collapse empty output → metadata flag, no cloud call without policy

---

### PR5 — Task plan + verify-driven outer loop

**Title:** Plan state and verify-driven coding outer loop  
**Size:** L  
**Depends on:** PR2, PR3  
**Branch:** `feature/harness-plan-verify`

**Changes:**

- `TaskPlan` structure: id, steps (pending/done/failed), acceptance criteria, verify command optional.
- Runtime outer loop:
  1. Ensure plan (model or heuristic for coding prompts)
  2. Inner ToolLoop for next step
  3. Verify (test_runner / cargo test / configured check) when plan says so
  4. On fail: inject failure into context, continue until budget
  5. On success: mark step done; stop when plan complete
- Expose plan snapshot via MessageDelta metadata and/or AG-UI if trivial; at minimum `@context` / context_snapshot on server.
- Wire CAW-001/002/004.

**Files:**

- Create: `crates/arkavo-agent-runtime/src/plan.rs`
- Create: `crates/arkavo-agent-runtime/src/verify.rs`
- Edit: runtime loop orchestration
- Edit: chat + server adapters to pass coding mode flag
- Edit: `specs/arkavo-edge/coding-agent-workflow.spec.yaml`
- Optional: `crates/arkavo-mcp-tools/src/test_runner.rs` integration only

**Acceptance:**

- Coding prompt with failing test fixture: agent edits and re-runs until green or budget stop with clear reason.
- Plan survives one compaction cycle (steps still present).
- Non-coding chat does not force plan ceremony (mode detect or user flag).

**Tests:**

- Fixture repo: fail → fix ≤ N steps
- Budget stop leaves plan state queryable
- CAW scenarios automated where possible with fake test tool

---

### PR6 — Automatic working memory retrieval

**Title:** Auto memory inject and durable store  
**Size:** M  
**Depends on:** PR3, PR5 (plan keys optional)  
**Branch:** `feature/harness-auto-memory`

**Changes:**

- Before each user turn / cycle: retrieve top-k memories + recent case lessons (reuse learning bus / memory search APIs).
- After successful plan completion: store durable summary (goal, files touched, outcome).
- Memory injection is system/context block, not only optional MCP tool.
- Keep MCP store/search tools for explicit control.

**Files:**

- Edit: `crates/arkavo-agent-runtime/src/memory_policy.rs` (new)
- Edit: chat_session + conductor injection points
- Edit: `crates/arkavo-memory` if retrieval API needs a clean call
- Edit: `specs/arkavo-edge/memory.spec.yaml` (auto-retrieve scenarios)

**Acceptance:**

- Second session turn can recall fact stored after prior success without model calling store_memory.
- Retrieval failure is soft (log + continue), never blocks chat.

**Tests:**

- Store then new session with same store → prompt contains memory snippet
- Retrieval error does not fail loop

---

### PR7 — Stuck / no-progress hard stops

**Title:** Tool stuck-loop detection and hard stop  
**Size:** S–M  
**Depends on:** PR1 (better after PR2)  
**Branch:** `feature/harness-stuck-detection`

**Changes:**

- Detectors:
  - Same tool + same args ≥ 3 → replan once, then Stuck stop
  - No FS/git/test state change after K iterations → Stuck
  - Promote ToolMemory action-variety signals to hard stops with user-visible reason
- Distinct from text OutputLoop (CRIT-010).
- Spec STUCK-001/002 (add to agent-tool-loop or small stuck spec).

**Files:**

- Create: `crates/arkavo-agent-runtime/src/stuck.rs`
- Edit: ToolMemory or observe side effects via tool results
- Edit: ATL / critic specs cross-ref

**Acceptance:**

- Infinite “observe same” loop terminates with StopReason::Stuck and message listing repeated tool.
- Legitimate repeated reads of different files do not false-positive (args must match).

**Tests:**

- Synthetic same-args thrash → stop
- Different args same tool → continue

---

### PR8 — Permission modes and sandbox defaults

**Title:** Agent permission modes for tools  
**Size:** M  
**Depends on:** PR1; integrate with PR2 chat  
**Branch:** `feature/harness-permissions`

**Changes:**

- Product modes: `ReadOnly | Ask | Workspace | Full`
- Layer on existing `retain_granted` / `tool_call_permitted` and `ToolSandbox`
- Default interactive: `Ask` for write/shell/network; coding sandbox for shell when Docker/OS available
- Config via env / agent config / CLI flag (`--permission-mode`)
- CAW-003

**Files:**

- Create: `crates/arkavo-agent-runtime/src/permissions.rs`
- Edit: `crates/arkavo-mcp-tools/src/sandbox.rs` (ensure execute path used)
- Edit: CLI chat flags, agent config types
- Edit: `specs/arkavo-edge/mcp-tools.spec.yaml` / CAW-003 / new permission scenarios

**Acceptance:**

- ReadOnly rejects write_file / shell_exec before execution.
- Ask mode emits approval-required event (or blocks with clear error if no UI yet).
- Workspace mode runs shell under ToolSandbox when strategy ≠ None.

**Tests:**

- Mode matrix unit tests
- Sandbox strategy detect smoke test

---

### PR9 — Step transcript and cost timeline

**Title:** Unified agent step transcript  
**Size:** M  
**Depends on:** PR1 events; richer after PR2–5  
**Branch:** `feature/harness-transcript`

**Changes:**

- Persist/stream `StepEvent` sequence per session: model, tokens, tools, stop reason, quality, spend plane decisions.
- Surface in chat deltas / AG-UI metrics if already wired; file export optional (`~/.arkavo/sessions/<id>/transcript.jsonl`).
- Minimal OTLP span per step if observability feature allows; if not, structured tracing fields only (no fake OTLP).
- Document how to debug a failed coding run from transcript.

**Files:**

- Edit: `crates/arkavo-agent-runtime/src/events.rs`
- Edit: chat_session / AG-UI handlers as needed
- Edit: `crates/arkavo-observability` only if real spans
- Create: short `docs/agent-transcript.md`

**Acceptance:**

- Multi-step chat produces ordered transcript with ≥1 event per tool iteration.
- Transcript includes final StopReason.
- No secrets (API keys) in transcript (redact).

**Tests:**

- Redaction unit test
- Event count matches iterations

---

### PR10 — CLI and server cutover; delete dual paths

**Title:** Cut CLI and server loops over to agent-runtime  
**Size:** L  
**Depends on:** PR2–PR9 core pieces (minimum PR2, PR3, PR7, PR1)  
**Branch:** `feature/harness-cutover`

**Changes:**

- `tool_integration::process_with_tools` becomes thin frontend over runtime.
- `conductor_tool_loop` / parallel planner call runtime (parallel may wrap multiple runtime runs or keep three-track as strategy plugin).
- Remove duplicated condense/stuck/max-iter constants from three places; single config source.
- Align defaults: document chat vs autonomous default budgets in one table in docs + Agents.md.
- Update CAPABILITIES.md harness description; retire stale progressive-disclosure “Phase 2” claims.

**Files:**

- Edit: `crates/arkavo-cli/src/tool_integration.rs`
- Edit: `crates/arkavo-server/src/server/conductor_tool_loop.rs`
- Edit: `crates/arkavo-server/src/server/conductor_parallel.rs`
- Edit: `docs/PROGRESSIVE_TOOL_DISCLOSURE.md`
- Edit: `docs/CAPABILITIES.md`
- Edit: UAL specs clear WIP

**Acceptance:**

- One implementation of max_steps / condense / stuck (grep shows single source).
- Server overnight-style unit tests still pass.
- CLI process_with_tools behavior parity tests.

**Tests:**

- Parity fixtures: same mock tools, same stop outcomes across chat/CLI/server adapters
- Full `cargo nextest` on touched crates

---

### PR11 — Coding subagent explore / implement / verify (optional)

**Title:** Default coding subagent split  
**Size:** L  
**Depends on:** PR5, PR8, PR10  
**Branch:** `feature/harness-coding-subagents`

**Changes:**

- Three internal roles (same process, restricted registries):
  - Explore: read-only tools
  - Implement: edit tools under permission mode
  - Verify: test/build tools only
- Orchestrated by TaskPlan outer loop; not full mesh/A2A unless already specialized.
- Optional later: map to SwarmKit roles.

**Files:**

- Edit: agent-runtime plan/verify
- Edit: registry filtering
- Specs: CAW extension

**Acceptance:**

- Explore cannot write files (enforced).
- Implement cannot run arbitrary network without permission.
- Verify-only phase cannot edit sources.

**Tests:**

- Registry filter per phase
- End-to-end fixture with three phases

---

## Cross-cutting PR checklist (every PR)

Per Agents.md / project rules:

```bash
cargo fmt -- --check
cargo build -q
cargo clippy -- -D warnings
# relevant crate tests
cargo nextest run -p arkavo-agent-runtime   # when exists
cargo nextest run -p arkavo-protocol
cargo nextest run -p arkavo-server --lib
```

- No Conventional Commits; short PR titles ≤ 60 chars; one topic per PR.
- Bump semver in workspace `Cargo.toml` when a feature completes (not every intermediate PR unless releasing).
- Commit `Cargo.lock` when `Cargo.toml` changes.
- Every bug fix gets a regression test.
- Security-sensitive tool/permission PRs: run relevant security scripts if touching DLP/shell.

## Suggested Graphite / branch stacks

**Stack A — Foundation (serial):**  
`PR0 → PR1 → PR2 → PR4 → PR5`

**Stack B — Context/memory (after PR1):**  
`PR1 → PR3 → PR6`

**Stack C — Safety (after PR1/PR2):**  
`PR1 → PR7 → PR8`

**Stack D — Finish:**  
`PR9 → PR10 → (PR11)`

## Effort estimate (engineering days, single senior)

| PR | Est. days | Risk |
|----|-----------|------|
| PR0 | 1–2 | Low |
| PR1 | 3–5 | Medium (extract without drift) |
| PR2 | 3–5 | High (chat UX/protocol) |
| PR3 | 3–4 | Medium |
| PR4 | 2–3 | Medium |
| PR5 | 4–6 | High |
| PR6 | 2–3 | Low–medium |
| PR7 | 1–2 | Low |
| PR8 | 2–3 | Medium (UX for Ask) |
| PR9 | 2–3 | Low |
| PR10 | 3–5 | High (cutover) |
| PR11 | 4–6 | Medium |
| **Total** | **~30–47** | |

MVP user-visible win: **PR0 + PR1 + PR2 + PR4** (~9–15 days) — multi-step quality chat.

## Success metrics

| Metric | Before | Target after MVP (PR2+PR4) | Target after PR10 |
|--------|--------|----------------------------|-------------------|
| Max tool steps on chat | 1 | 25 soft | same + plan/verify |
| Shared loop implementations | 3 | 2 (chat+runtime; server wrap) | 1 |
| Compaction | cold drop | semantic on chat | all paths |
| Stuck thrash | soft/none on chat | hard stop | hard stop everywhere |
| Specs for control loop | ~0 | ATL+UAL+CAW | all green / not WIP |
| Coding task: edit→test→fix | often incomplete | multi-step possible | verify-driven default |

## Open questions (resolve before PR5 / PR8)

1. **Ask-mode UX without AG-UI:** block with error text vs stdin prompt vs auto-deny?  
   Recommendation: block with structured delta `permission_required` for UI; CLI stdin prompt.
2. **Default permission mode for local CLI:** Ask vs Full?  
   Recommendation: Full for local trusted workspace; Ask when network-facing or agent mesh.
3. **Parallel three-track conductor:** fold into runtime strategies or keep as server-only optimizer?  
   Recommendation: keep server plugin in PR10; do not block chat MVP.
4. **Grok `previous_response_id` chaining:** in-scope for harness?  
   Recommendation: out of scope until PR10; separate provider PR.

## Execution notes

- Prefer Graphite stacks matching Stack A for the first ship.
- Do not expand chat window without PR3 compaction (OOM / context overflow risk).
- When A2A realignment lands, keep dual-path matrix tests from UAL-001/008 green as a merge gate.
- This document is the source of truth for sequencing; update checkboxes in PRs, not by inventing parallel plans.

## Related docs

- `docs/superpowers/specs/2026-04-05-unified-agent-loop-design.md`
- `docs/three-plane-router.md`
- `docs/coding-agent-toolset.md`
- `docs/PROGRESSIVE_TOOL_DISCLOSURE.md`
- `docs/a2a/realignment-scope.md`
- `docs/grok-4.5-support-plan.md` (provider; not harness depth)
