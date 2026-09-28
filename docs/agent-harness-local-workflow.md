# Agent Harness — Local-Only Development Workflow

**Date:** 2026-07-11  
**Status:** Active  
**Pairs with:** [`docs/agent-harness-pr-plan.md`](agent-harness-pr-plan.md)  
**Constraint:** Do **not** use GitHub CI during development. No intermediate PRs. One final PR.

## Goal

Ship the agent harness unification as **one GitHub PR** after all work is complete, while keeping quality high by:

1. Staying on a **single long-lived local branch**
2. Running **targeted tests** after every phase commit
3. Running **full local regression** only at defined checkpoints
4. **Pushing once** when ready for the final PR (CI runs then and only then)

This avoids multi-hour Feature workflow cost on every incremental PR (`feature.yaml` matrix: version, fmt, clippy, many package tests, deny, audit, multi-OS builds, etc.).

## Non-goals

- Disabling or changing GitHub CI configuration for the repo
- Skipping quality bars — only **when** they run changes (local, staged)
- Force-pushing rebased history to `main` or bypassing required checks on the final PR
- Intermediate Graphite / stacked PRs (the PR plan’s “PR0–PR11” become **local phases**, not remote PRs)

## Mapping: plan PRs → local phases

| Plan PR | Local phase id | Meaning |
|---------|----------------|---------|
| PR0 | `P0` | Specs + ID hygiene |
| PR1 | `P1` | `arkavo-agent-runtime` extract |
| PR2 | `P2` | Chat multi-step loop |
| PR3 | `P3` | Semantic compaction |
| PR4 | `P4` | Quality gate on chat |
| PR5 | `P5` | Plan + verify outer loop |
| PR6 | `P6` | Auto memory |
| PR7 | `P7` | Stuck detection |
| PR8 | `P8` | Permission modes |
| PR9 | `P9` | Transcript |
| PR10 | `P10` | CLI/server cutover |
| PR11 | `P11` | Subagents (optional) |

MVP shippable locally: **P0–P4**. Full plan: **P0–P10** (+ optional P11).

## Branch rules

```bash
# Create once from latest main (do this before any harness work)
git fetch origin main
git checkout main
git pull --ff-only origin main
git checkout -b feature/agent-harness

# During development: never push
# git push is forbidden until Final Gate (below)

# Optional: track progress with annotated tags (local only)
git tag harness-p0   # after P0
git tag harness-cp1  # after Checkpoint 1, etc.
```

**Rules:**

| Rule | Detail |
|------|--------|
| Branch name | `feature/agent-harness` (one topic; final PR title stays short) |
| Remote | **No push** until Final Gate |
| Commits | Small, local, frequent; no Conventional Commits |
| Rebase on main | Only at checkpoints if main moved; prefer rebase when clean |
| WIP | Use WIP commits freely; squash or leave readable history before final PR |
| Secrets | Never commit API keys; same as always |

If you need a remote backup without CI, use a **private** fork or `git bundle` — not `origin` on arkavo-edge (any PR/push to the main repo can trigger workflows).

```bash
# Offline backup example (no GitHub)
git bundle create ~/backups/agent-harness-$(date +%Y%m%d).bundle feature/agent-harness
```

## Test tiers (local)

### Tier T — Targeted (every phase commit)

Fast feedback. Only packages touched by that phase.

```bash
# Always (cheap)
cargo fmt -- --check
cargo build -q -p <touched-crates...>

# Prefer nextest when available
cargo nextest run -p <crate> --lib
# or
cargo test -p <crate> --lib
```

### Tier C — Checkpoint (full local harness regression)

Run at **Checkpoint** markers below. Mirrors the expensive parts of CI that matter for this work, **on one machine**, debug builds only (no `--release` per Agents.md).

### Tier F — Final Gate (before first push + PR)

Everything in Tier C **plus** security scripts + version bump + locked checks that CI will enforce.

---

## Tier T recipes by phase

Run from repo root. Add crates if the phase expands.

### P0 — Specs only

```bash
# Schema validate if ajv available; otherwise visual review
npx --yes ajv-cli validate -s specs/schema.json -d "specs/arkavo-edge/agent-tool-loop.spec.yaml" 2>/dev/null || true
npx --yes ajv-cli validate -s specs/schema.json -d "specs/arkavo-edge/unified-agent-loop.spec.yaml" 2>/dev/null || true
npx --yes ajv-cli validate -s specs/schema.json -d "specs/arkavo-edge/coding-agent-workflow.spec.yaml" 2>/dev/null || true

# ID uniqueness smoke
rg -o 'id: (ATL|UAL|CAW|ORCH|TASK)-[0-9]+' specs/arkavo-edge | sort | uniq -d
# expect empty
```

### P1 — agent-runtime + server delegate

```bash
cargo fmt -- --check
cargo build -q -p arkavo-agent-runtime -p arkavo-server
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-server --lib
# if nextest missing:
# cargo test -p arkavo-agent-runtime && cargo test -p arkavo-server --lib
```

### P2 — chat multi-step

```bash
cargo fmt -- --check
cargo build -q -p arkavo-protocol -p arkavo-agent-runtime -p arkavo-cli
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-protocol
cargo nextest run -p arkavo-cli --lib
```

### P3 — compaction / context / session

```bash
cargo fmt -- --check
cargo build -q -p arkavo-context -p arkavo-session -p arkavo-agent-runtime -p arkavo-protocol
cargo nextest run -p arkavo-context
cargo nextest run -p arkavo-session --lib
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-protocol
```

### P4 — quality on chat

```bash
cargo fmt -- --check
cargo build -q -p arkavo-router -p arkavo-protocol -p arkavo-agent-runtime -p arkavo-critic
cargo nextest run -p arkavo-router --no-default-features --features llm-remote,gemini
cargo nextest run -p arkavo-critic
cargo nextest run -p arkavo-protocol
cargo nextest run -p arkavo-agent-runtime
```

### P5 — plan + verify

```bash
cargo fmt -- --check
cargo build -q -p arkavo-agent-runtime -p arkavo-protocol -p arkavo-mcp-tools
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-mcp-tools
cargo nextest run -p arkavo-protocol
```

### P6 — memory

```bash
cargo fmt -- --check
cargo build -q -p arkavo-memory -p arkavo-agent-runtime -p arkavo-protocol
cargo nextest run -p arkavo-memory --lib
cargo nextest run -p arkavo-agent-runtime
```

### P7 — stuck detection

```bash
cargo fmt -- --check
cargo build -q -p arkavo-agent-runtime
cargo nextest run -p arkavo-agent-runtime
```

### P8 — permissions / sandbox

```bash
cargo fmt -- --check
cargo build -q -p arkavo-agent-runtime -p arkavo-mcp-tools -p arkavo-cli
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-mcp-tools
cargo nextest run -p arkavo-cli --lib
```

### P9 — transcript

```bash
cargo fmt -- --check
cargo build -q -p arkavo-agent-runtime -p arkavo-protocol
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-protocol
```

### P10 — cutover CLI + server

```bash
cargo fmt -- --check
cargo build -q -p arkavo-cli -p arkavo-server -p arkavo-agent-runtime
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-cli --lib
cargo nextest run -p arkavo-server --lib
cargo nextest run -p arkavo-protocol
```

### P11 — subagents (optional)

```bash
cargo fmt -- --check
cargo build -q -p arkavo-agent-runtime -p arkavo-server
cargo nextest run -p arkavo-agent-runtime
cargo nextest run -p arkavo-server --lib
```

**Clippy during Tier T:** optional after small edits; **required** at every Checkpoint and Final Gate.

```bash
# Narrow clippy when iterating
cargo clippy -p arkavo-agent-runtime -- -D warnings
```

---

## Checkpoint schedule

| Checkpoint | After phase | What you prove | Command set |
|------------|-------------|----------------|-------------|
| **CP0** | P0 | Specs sane | Tier T P0 only |
| **CP1** | P1 | Runtime extract no server regression | Tier C-Harness |
| **CP2** | P2 + P4 | Chat multi-step + quality MVP | Tier C-Harness |
| **CP3** | P3 + P5 | Compaction + plan/verify | Tier C-Harness |
| **CP4** | P6–P9 | Memory, stuck, perms, transcript | Tier C-Harness |
| **CP5** | P10 (+P11) | Single loop; cutover complete | Tier C-Harness |
| **Final** | All done | Ship-ready; then push + open PR | Tier F |

MVP path: **P0 → P1 → CP1 → P2 → P4 → CP2 → Final** (skip P3–P11 if shipping MVP only; document leftover as follow-up).

Full path: all phases, **CP1–CP5**, then **Final**.

---

## Tier C — Checkpoint harness regression

One script-shaped sequence. Run at CP1–CP5. Expect **tens of minutes**, not hours of multi-OS CI.

```bash
#!/usr/bin/env bash
# Save as: scripts/harness-checkpoint.sh  (optional; or run by hand)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "== fmt =="
cargo fmt -- --check

echo "== build (debug, harness crates) =="
cargo build -q -p arkavo-agent-runtime \
  -p arkavo-protocol -p arkavo-router -p arkavo-context \
  -p arkavo-session -p arkavo-mcp-tools -p arkavo-memory \
  -p arkavo-critic -p arkavo-budget -p arkavo-cli -p arkavo-server \
  2>/dev/null || cargo build -q  # fallback if agent-runtime not yet in workspace

echo "== clippy (harness-related packages) =="
for p in arkavo-agent-runtime arkavo-protocol arkavo-router arkavo-context \
         arkavo-session arkavo-mcp-tools arkavo-memory arkavo-critic \
         arkavo-cli arkavo-server; do
  cargo metadata --format-version 1 --no-deps 2>/dev/null | grep -q "\"$p\"" || continue
  case "$p" in
    arkavo-router)
      cargo clippy -p "$p" --lib --bins --no-default-features --features llm-remote,gemini -- -D warnings
      ;;
    arkavo-cli)
      cargo clippy -p "$p" --lib --bins --no-default-features \
        --features memory,mdns,mcp-tools,llm-remote,web-ui -- -D warnings
      ;;
    *)
      cargo clippy -p "$p" --lib --bins -- -D warnings 2>/dev/null \
        || cargo clippy -p "$p" -- -D warnings
      ;;
  esac
done

echo "== unit/integration (nextest preferred) =="
run() { cargo nextest run "$@" 2>/dev/null || cargo test "$@"; }

run -p arkavo-agent-runtime || true
run -p arkavo-context
run -p arkavo-session --lib
run -p arkavo-memory --lib
run -p arkavo-budget
run -p arkavo-critic
run -p arkavo-mcp-tools
run -p arkavo-protocol
run -p arkavo-router --no-default-features --features llm-remote,gemini
run -p arkavo-cli --lib --no-default-features --features memory,mdns,mcp-tools,llm-remote,web-ui
run -p arkavo-server --lib

echo "== checkpoint OK =="
```

**Notes:**

- Do **not** run multi-target release builds or full workspace `cargo test` at every checkpoint unless something failed mysteriously.
- If a package is not yet a workspace member (before P1), the loop skips/fails soft as shown.
- Prefer `cargo nextest run` when installed (`cargo install cargo-nextest`).

**Checkpoint exit criteria:**

- [ ] All Tier C commands exit 0
- [ ] Local tag `harness-cpN` created
- [ ] Short note in commit message: `Checkpoint N: <phases included>`
- [ ] If main advanced significantly: rebase, re-run Tier C

```bash
git fetch origin main
git rebase origin/main   # only at checkpoint, when clean
# fix conflicts, re-run Tier C
git tag -f harness-cp2   # or create new tag
```

---

## Tier F — Final Gate (once, before push)

Run only when the local branch is feature-complete for the ship set (MVP or full).

### F1 — Workspace quality

```bash
cargo fmt -- --check
cargo build -q
cargo clippy -- -D warnings   # full workspace if feasible; else Tier C clippy + any new crates
```

If full-workspace clippy is too heavy on this machine, at minimum:

- All harness packages from Tier C with `-D warnings`
- `cargo build -q` for default members / full workspace

### F2 — Harness regression (Tier C again)

Re-run the full checkpoint script after the last code change.

### F3 — Security (Agents.md pre-push; required before push)

```bash
cargo test -p arkavo-protocol --test security_vulnerabilities
cargo test -p arkavo-cli --lib mock_provider::

./tests/e2e_security_test.sh
./tests/security_cli_test.sh    # needs local models if script requires them
./tests/dlp_pii_security_test.sh
```

If a security script needs models/hardware you lack, document skip reason in the PR body and run every script that **can** run. Do not push knowing a runnable security test is red.

### F4 — Version + lockfile (CI will enforce)

```bash
# Bump workspace version in root Cargo.toml (semver for feature completion)
# Commit Cargo.lock if any Cargo.toml changed

git fetch origin main
git show origin/main:Cargo.toml | grep '^version ='
grep '^version =' Cargo.toml | head -1
# PR version must be > main (feature.yaml version-check)
```

### F5 — Optional local “CI shape” (still single machine)

Mirrors high-value jobs from `feature.yaml` without multi-OS:

```bash
cargo test --locked -p arkavo-llm --lib --no-default-features --features llm-remote
cargo test --locked -p arkavo-protocol
cargo test --locked -p arkavo-router --no-default-features --features llm-remote,gemini
cargo test --locked -p arkavo-cli --lib --no-default-features --features memory,mdns,mcp-tools,llm-remote,web-ui

# If installed:
cargo deny check   # or cargo-deny
cargo audit
```

### F6 — History hygiene

```bash
# Optional: interactive rebase to clean noise commits (local only, never pushed yet)
# git rebase -i origin/main

git log --oneline origin/main..HEAD
```

Prefer a **readable linear history** of phase commits (`P0 … P10`) over one megacommit; reviewers can still read one PR.

### F7 — Push and open the single PR

```bash
git push -u origin feature/agent-harness

gh pr create --base main --head feature/agent-harness \
  --title "Unified agent harness runtime" \
  --body-file docs/agent-harness-pr-body.md   # create when shipping; see template below
```

**After push:** GitHub CI runs **once** on this PR. Fix CI failures with small follow-up commits on the same branch (expected), not by re-splitting into many PRs.

---

## Final PR body template

Create `docs/agent-harness-pr-body.md` at ship time (or paste into `gh pr create`):

```markdown
## Summary
Unifies chat / CLI / server agent tool loops onto a shared runtime and deepens
the default interactive coding path (multi-step tools, quality, compaction, …).

## Scope
- Phases included: P0–P? (list)
- Out of scope: (list deferred phases)

## Local verification (no intermediate CI)
- [ ] Tier C checkpoints: CP1 … CP?
- [ ] Tier F: fmt, build, clippy, harness tests
- [ ] Security scripts: (list pass / skip+reason)
- [ ] Version bumped vs main

## Test plan for reviewers
- cargo nextest run -p arkavo-agent-runtime
- cargo nextest run -p arkavo-protocol
- Multi-step chat smoke: (prompt)

## Risk
- Chat protocol / MessageDelta compatibility
- Server conductor behavior parity
```

## Daily loop (agent or human)

```text
1. Implement one phase (or slice of a phase)
2. cargo fmt
3. Tier T for that phase
4. Commit on feature/agent-harness
5. If at Checkpoint boundary → Tier C + tag
6. Do not push
7. Repeat until ship set done → Tier F → push → one PR
```

## What not to do

| Avoid | Why |
|-------|-----|
| Opening PR early “to run CI” | Defeats the purpose; Feature workflow is the cost |
| Pushing WIP to `origin` on this repo | Triggers PR CI if a PR exists; pollutes history |
| Full `cargo test` workspace after every file | Slow; use Tier T |
| `--release` builds while iterating | Agents.md; debug only |
| Skipping Tier F security because “CI will catch it” | CI may not run all local security scripts the same way; fix before push |
| Multiple feature branches for P0–P11 | Reintroduces merge/CI tax; phases are commits/tags only |

## Progress tracking (local)

Optional checklist file (not required for product):

```bash
# docs/agent-harness-progress.md  (local notes only; can commit or not)
```

| Phase | Commit | Tier T | Checkpoint |
|-------|--------|--------|------------|
| P0 | | | CP0 |
| P1 | | | CP1 |
| P2 | | | |
| P4 | | | CP2 (MVP) |
| … | | | |

## Relation to `agent-harness-pr-plan.md`

- That doc’s **PR0–PR11** remain the **implementation design and acceptance criteria**.
- This doc replaces **remote PR sequencing** with **local phases + checkpoints + one final PR**.
- Do not open 12 GitHub PRs. Do implement in the same order (P0→P1→…) unless a dependency allows reordering (e.g. P7 after P1).

## Recovery

| Situation | Action |
|-----------|--------|
| Checkpoint fails | Fix on branch; do not tag until green |
| Main moved a lot | Rebase at next checkpoint; re-run Tier C |
| Need backup | `git bundle` or private remote; not origin PR |
| Final CI red | Small fix commits on same branch; re-run local repro of failing job |
| Scope cut for ship | Tag `harness-mvp`; Final Gate on P0–P4 only; leave P5+ for later branch |

## Success definition

1. All planned phases for the ship set implemented on `feature/agent-harness`
2. Every Checkpoint for that set green locally
3. Final Gate green locally
4. **One** PR opened; GitHub CI is the first remote verification
5. PR merges with CI green (fix-forward commits allowed after first push)
