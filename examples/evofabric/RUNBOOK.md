# EvoFabric Runbook

Step-by-step guide to testing EvoFabric code evolution.

## What This Demonstrates

- Typed AST operations (not raw text diffs)
- Semantic scope targeting (by function/type name, not line number)
- Critic verification pipeline (parse, apply, compile, test)
- Atomic git commits for recoverability

## Prerequisites

1. Build the binary:
   ```bash
   cd /path/to/arkavo-edge
   cargo build
   ```

2. For live mode only: a local LLM must be running (ministral-3b or similar)

## Offline Mode (No LLM Required)

### Step 1: Navigate to Example

```bash
cd examples/evofabric
```

### Step 2: Run the Offline Pipeline

```bash
./run_offline.sh
```

**What to watch for:**
- The pipeline reads `sample.rs` and applies `bundle.json`
- Three operations: add `#[inline]`, replace method body, add `#[must_use]`
- The rendered output is valid Rust (verified by re-parsing)

### Step 3: Observe Output

```
EvoFabric Offline Pipeline
==========================

Source file: sample.rs
OpBundle:    bundle.json

Operations: 3
Rationale:  Add inline hint, harden validation, mark pure function

--- Running AST Pipeline ---

[output showing the transformed source or test results]

Pipeline succeeded.
```

### Step 4: Run Integration Tests Directly

```bash
cargo test -p arkavo-evofabric --test end_to_end -- --nocapture
```

Expected: 8 tests pass covering all OpBundle operations.

## Live Mode (Requires Local LLM)

### Step 1: Verify Model is Running

```bash
curl -s http://localhost:8080/health | head -1
```

### Step 2: Run the Live Pipeline

```bash
./run.sh
```

Or with a custom instruction:

```bash
./run.sh "add #[must_use] to is_gpu_accelerated"
```

**What to watch for:**
- The agent reads the target source file
- LLM generates a JSON OpBundle
- OpBundle is parsed and applied to the AST
- Modified source is compiled in an isolated temp workspace
- If compilation and tests pass, a git commit is created

### Step 3: Verify the Commit

```bash
git log --oneline -3
```

Look for a commit message starting with `evofabric:`.

```bash
git diff HEAD~1
```

Shows the exact change made by the agent.

### Step 4: Revert if Needed

```bash
git revert HEAD
```

## Troubleshooting

### Offline Pipeline Fails

```bash
# Run the integration tests directly
cargo test -p arkavo-evofabric --test end_to_end -v
```

### LLM Returns Invalid JSON

The `from_json()` parser handles common LLM output issues:
- Strips markdown code fences
- Removes trailing commas
- Clear error messages on parse failure

### Compilation Fails in Temp Workspace

The temp workspace uses symlinks for sibling crates. Check:
```bash
# Verify the workspace builds normally
cargo build -q
```

### No Git Commit Created

The pipeline only commits if both `cargo check` and `cargo test` pass in the temp workspace. Check the output for compiler or test errors.

## Architecture Notes

### Why AST Operations (Not Text Diffs)?

- **Semantic targeting**: Operations target functions by name, not line number
- **Composability**: Operations on disjoint subtrees are naturally commutative
- **Validation**: Each operation is validated at parse time (syntax errors caught early)
- **Determinism**: `prettyplease` renders consistent formatting

### Why a Temp Workspace?

- Modifications are verified before touching the real source
- The temp workspace gets its own `target/` directory (no mutation risk)
- Sibling crates are symlinked for fast setup
- Cold build cost is amortized across multiple evaluations

### Recovery Model

Every verified change is committed via git:
- `git log` provides a complete audit trail
- `git revert HEAD` undoes the last change
- If the process crashes between file write and commit, `git checkout -- <file>` recovers
