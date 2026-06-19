# SwarmKit PR-review — WS-A (manifest fold-in) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Vendor the `github-ops-kit` manifest + validation test (#626) onto `feature/swarmkit-pr-review`, with grants adjusted for this design: `dispatcher` gets the `github_pr_watch` poll grant; `pr_test_runner` gets the existing `test_run` tool (no model-regression eval gate).

**Architecture:** This is a fold-in only — no Rust crate changes. The manifest declares each role's purpose + tool-grant capability; the swarm decides behavior at runtime (no hard-coded test gate). The validation test asserts the manifest parses, validates, round-trips its kit-id, and carries the adjusted grants.

**Tech Stack:** YAML manifest, `arkavo-swarmkit` validation test (`cargo test`).

## Global Constraints

- No `--release` builds; use debug.
- No clippy warnings: `cargo clippy -p arkavo-swarmkit --tests -- -D warnings`.
- No Conventional Commits prefixes (no `feat:`/`fix:`). Use the exact commit message below including its `Co-Authored-By` / `Claude-Session` trailer.
- The manifest declares **capability (tool grants), not procedure** — do not add instructions that hard-code a fixed test routine; the swarm decides whether/what to test.
- `github_pr_watch` and `test_run` are referenced as grant strings; `github_pr_watch` (the tool) lands in WS-B. The manifest `validate()` checks structure, not tool existence, so this is fine.

---

### Task 1: Vendor the github-ops-kit manifest + validation test (#626), adjust grants

**Files:**
- Create (vendored): `examples/github-ops-kit/github-ops-kit.swarmkit.yaml`, `crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs` (from `origin/feature/github-ops-kit`)
- Modify: the vendored manifest (two grant changes) + the vendored test (assertions)
- Test: `crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs`

**Interfaces:**
- Produces: a validated `github-ops-kit` manifest whose `dispatcher` grants `github_pr_watch` and whose `pr_test_runner` grants `test_run` (replacing #626's `run_eval`).

- [ ] **Step 1: Vendor the manifest + test from the #626 branch**

```bash
cd /Users/arkavo/Projects/arkavo/arkavo-edge
git fetch origin feature/github-ops-kit --quiet
git checkout origin/feature/github-ops-kit -- examples/github-ops-kit crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs
```

- [ ] **Step 2: Replace the `pr_test_runner` gate grant `run_eval` → `test_run`**

In `examples/github-ops-kit/github-ops-kit.swarmkit.yaml`, under the `pr_test_runner` role's `mcp_tools`, change the granted tool from `run_eval` to `test_run` (the existing test-runner tool). The `server` label is the logical MCP server name already used by the other tool grants in this manifest — match that existing label (do not invent a new one):

```yaml
      - server: "<same-server-label-as-the-other-tool-grants>"
        tools: ["test_run"]
        auth: none
```

If the `pr_test_runner` role's `skills`/`instructions` text names the eval gate or "run_eval" specifically, soften it to describe the role's *purpose* (assess and run whatever tests the PR warrants) rather than a fixed routine — the swarm decides whether/what to test. Do not add a hard-coded "always run X" instruction.

- [ ] **Step 3: Add the `github_pr_watch` grant to the `dispatcher` role**

In the `dispatcher` role's `mcp_tools`, add the PR-watch grant (the tool ships in WS-B; the manifest validates structure only). Use the same `server` label as the dispatcher's existing tool grants:

```yaml
      - server: "<same-server-label-as-dispatcher's-other-grants>"
        tools: ["github_pr_watch"]
        auth: delegated
```

- [ ] **Step 4: Update the validation test assertions**

In `crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs`: change any assertion referencing `run_eval` to `test_run`, and add an assertion that `dispatcher` grants `github_pr_watch`. Adapt to the test's existing helper for finding a role's granted tools (read the vendored test to match its style). Example:

```rust
// pr_test_runner has the general test-runner capability (swarm decides usage)
assert!(role_tools(&manifest, "pr_test_runner").contains(&"test_run".to_string()));
// dispatcher monitors PRs via the poll tool (tool lands in WS-B)
assert!(role_tools(&manifest, "dispatcher").contains(&"github_pr_watch".to_string()));
// the eval gate is gone
assert!(!role_tools(&manifest, "pr_test_runner").contains(&"run_eval".to_string()));
```

- [ ] **Step 5: Run the validation test**

Run: `cargo test -p arkavo-swarmkit --test github_ops_kit_validates`
Expected: PASS (parses, cross-block `validate()`, kit-id round-trip, least-privilege boundaries, the adjusted grant assertions).

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy -p arkavo-swarmkit --tests -- -D warnings`
Expected: clean.

```bash
git add examples/github-ops-kit crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs
git commit -m "Vendor github-ops-kit manifest + validation test (from #626)

Adjusts grants for this branch: pr_test_runner gets the existing test_run tool
(no model-regression eval gate — out of scope for normal PR review; the swarm
decides whether/what to test); dispatcher gets the github_pr_watch poll grant
(tool lands in WS-B).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

## Self-Review

**Spec coverage (WS-A scope):**
- "Fold #626 manifest + validation test" → Task 1 (vendor). ✓
- "dispatcher gets github_pr_watch; pr_test_runner gets test_run (no eval gate)" → Task 1 steps 2–4. ✓
- "Roles declare capability, not procedure" → Task 1 step 2 (soften any hard-coded routine). ✓

**Placeholder scan:** The `<same-server-label-...>` placeholders are deliberate — the implementer must match the manifest's existing `server` label rather than invent one; the step says so explicitly. No vague "handle errors" or TBD steps.

**Type consistency:** Tool grant strings `github_pr_watch` / `test_run` consistent between manifest (steps 2–3) and test (step 4).
