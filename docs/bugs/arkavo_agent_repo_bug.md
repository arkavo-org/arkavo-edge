# Bug: arkavo chat cannot analyze current git repo (missing tools/feature-gate) and returns generic responses

**Labels:** bug, P1, arkavo-cli, agent  
**Branch observed:** fix/accumulated-bug-fixes  
**Last known change on branch:** Removal of deprecated --no-tui flag references; release notes RELEASE_NOTES_0.28.1.md added

---

## Summary

The Arkavo agent invoked via `arkavo chat` fails to perform repository-aware tasks (e.g., reporting current branch, last commit, listing crates). Instead, it emits generic LLM responses and/or tool-less "advice." `arkavo model list` also indicates feature gating: "Model management requires the 'local' feature to be enabled."

This prevents the agent from answering even basic repo questions and blocks several dev flows that rely on MCP/git tools.

---

## Impact

- **High** for day-to-day developer workflows that expect `arkavo chat` to introspect the local repo and answer codebase questions.
- Blocks automation we planned to accumulate on `fix/accumulated-bug-fixes` (e.g., repo-aware checks, quick branch/commit summaries).
- Undermines confidence in agent tooling readiness.

---

## Environment

- **Platform:** macOS (Apple Terminal)
- **Binary:** `./target/debug/arkavo`
- **Network:** Mixed (some runs included a timeout wrapper to avoid hangs)
- **Build:** `cargo check` passes; not built with `--release` for speed during iteration.

Note: precise OS/toolchain versions can be added by the assignee in comments if needed.

---

## Steps to Reproduce

From the repo root (on `fix/accumulated-bug-fixes`):

1. **Ask for branch + recent changes:**
   ```bash
   ARKAVO_NO_TERMINAL_RELAUNCH=1 timeout 10 ./target/debug/arkavo chat \
     --prompt "What is the current git branch and what recent changes have been made to this repo?"
   ```
   
   **Actual output (excerpt):**
   ```
   Error: ```
   git fetch
   
   Explanation:
   • git fetch ...
   ```
   (Repeats "git fetch" explanation without reporting current branch/commit.)

2. **Ask to list crates:**
   ```bash
   ARKAVO_NO_TERMINAL_RELAUNCH=1 timeout 5 ./target/debug/arkavo chat \
     --prompt "List the Rust crates in this project"
   ```
   
   **Actual output (excerpt):**
   ```
   Okay, I'm ready to help you with Rust. I'm excited to learn and assist you.
   ```

3. **Ask for last commit message:**
   ```bash
   ARKAVO_NO_TERMINAL_RELAUNCH=1 timeout 5 ./target/debug/arkavo chat \
     --prompt "What was the last git commit message?"
   ```
   
   **Actual output (excerpt):**
   ```
   I am a large language model.
   ```

4. **List models:**
   ```bash
   ./target/debug/arkavo model list
   ```
   
   **Actual output:**
   ```
   Model management requires the 'local' feature to be enabled.
   ```

The outputs indicate the agent lacks repo/tool access and model management under the current build configuration.

---

## Expected Behavior

- `arkavo chat` detects and uses repo-aware tools (MCP shell/git, fs, or a dedicated Git MCP server/libgit2).
- It should correctly answer:
  - Current branch name
  - Last commit message (subject + short sha)
  - List of Rust crates/workspaces found
- If a build-time feature is missing, the CLI should degrade gracefully (clear actionable error) or default to a working provider/config.

---

## Actual Behavior

- Generic, non–repo-aware LLM responses (no actual git inspection).
- Tool-less "advice" (e.g., repeating git fetch explanation) instead of executing tools.
- Feature-gated model management blocks local model usage (`local` feature not enabled).

---

## Notes & Findings (from the session)

- The binary appears to be built without features needed for local model mgmt and/or MCP tool wiring.
- The agent does not appear to dispatch to any git-aware tool (shell, libgit2, or MCP Git server).
- The recent `--no-tui` removal change is likely unrelated; this looks like feature gating/config.

---

## Acceptance Criteria

- `arkavo chat` can answer repo basics on a fresh debug build (no special flags):
  - Prints current git branch
  - Prints last commit subject + short sha
  - Lists detected Rust crates/workspaces (from Cargo metadata)
- If required features are disabled at build time, `arkavo chat` shows a single, clear error with how to enable them.
- Add an automated CLI smoke test (or BDD) that asserts the above in CI.
- Document the minimal config needed (e.g., MCP tools, providers, and feature flags).

---

## Suspected Root Causes (to investigate)

1. **Cargo feature-gate:** `local` feature is not enabled in the debug build; gating model mgmt (and possibly tool chains).
2. **MCP/tool wiring not active** in `arkavo chat` execution path:
   - No shell/git MCP tool registered
   - Missing fs tool for scanning crates, or not invoked
3. **Provider/model selection** defaults to a minimal generic model with no tools enabled.
4. **Timeout wrapper** may hide a hang, but present symptoms indicate missing tools rather than slow tools.

---

## Proposed Fix (initial direction)

- Ensure `arkavo-cli` enables/loads the git + fs tools by default in chat mode when inside a git repo.
- Make the toolchain provider-agnostic (local or remote) with a feature-independent fallback.
- Surface a clear message if repo tools aren't available, e.g.:
  ```
  "Repo analysis tools are disabled in this build. Enable with --features mcp,local or set ARKAVO_TOOLS=git,fs."
  ```
- Add a `--diagnostics` flag to print which tools/features/providers are active at runtime.

---

## Tasks

- [ ] Reproduce without timeout and capture logs.
- [ ] Add runtime diagnostics: list active features/tools/providers on startup of chat.
- [ ] Wire MCP git and fs tools in chat mode by default (when in a git repo).
- [ ] If Cargo features are required, document defaults and add graceful fallback messaging.
- [ ] Implement repo-introspection commands (branch/commit/crates) using:
  - Shell git (or libgit2) via MCP tool
  - `cargo metadata` for crates
- [ ] Add tests:
  - Unit test: parse/format branch & commit info
  - Integration test: ephemeral repo fixture validates outputs
  - CI smoke test target
- [ ] Update docs: troubleshooting page & dev quickstart.

---

## Test Plan

1. **Unit:** pure functions format branch/commit info; cargo metadata parsing.
2. **Integration:** create a temp git repo with 2 commits + minimal Cargo workspace; run `arkavo chat` queries; assert outputs.
3. **Manual (macOS, Linux):**
   - Run the four repro commands above (no timeout).
   - Confirm accurate outputs and absence of generic LLM filler.
4. **CI:** add a job using a small fixture repo to validate behavior on PRs.

---

## Additional Context / Artifacts

- **Branch:** `fix/accumulated-bug-fixes`
- **Recent change:** removal of `--no-tui` references (docs/tests/scripts).

**Assignee:** (TBD)  
**Milestone:** 0.28.1 (or next patch)  
**Depends on:** Tooling feature availability (local/MCP) if currently behind flags