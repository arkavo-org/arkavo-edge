# EvoFabric: AST-Level Code Evolution

Autonomous code modification through typed AST operations.

## What You'll Learn

- How EvoFabric proposes code changes as structured OpBundles
- The Critic verification pipeline (parse, apply, compile, test)
- Atomic git commits for recoverability

## Prerequisites

```bash
# Build Arkavo (from repo root)
cargo build
```

## Quick Start

### Test the AST Pipeline

Run all unit and integration tests to verify the pipeline:

```bash
./run_test.sh
```

This applies pre-built OpBundles to sample source files, verifies they parse correctly, and exercises all operation types.

### Run the Agent

Run the full agent pipeline with a local model:

```bash
./run.sh
```

The agent reads the target file, asks the model to propose an OpBundle, applies it to the AST, verifies compilation in an isolated workspace, and commits on success.

## What's Happening

### Test Pipeline

```
sample.rs ──► parse (syn) ──► apply OpBundle ──► render (prettyplease) ──► verify
```

### Agent Pipeline

```
[evofabric] task ──► read source ──► model proposes OpBundle ──► parse + apply
    ──► temp workspace ──► cargo check ──► cargo test ──► git commit
```

## Files

| File | Purpose |
|------|---------|
| `AGENTS.md` | Agent configuration |
| `run_test.sh` | AST pipeline verification (51 tests) |
| `run.sh` | Full agent pipeline with local model |
| `sample.rs` | Target source file for testing |
| `bundle.json` | Pre-built OpBundle for testing |
| `tasks.json` | Demo tasks for agent mode |

## Architecture

```
                    ┌──────────────┐
                    │  Local Model │  (proposes OpBundle)
                    └──────┬───────┘
                           │ JSON
                    ┌──────▼───────┐
                    │  OpBundle    │  (typed AST operations)
                    │  from_json() │
                    └──────┬───────┘
                           │
              ┌────────────▼────────────┐
              │  RustOp::apply_bundle() │  (syn AST mutation)
              └────────────┬────────────┘
                           │
              ┌────────────▼────────────┐
              │  render_file()          │  (prettyplease)
              └────────────┬────────────┘
                           │
              ┌────────────▼────────────┐
              │  TempWorkspace          │
              │  cargo check + test     │
              └────────────┬────────────┘
                           │
              ┌────────────▼────────────┐
              │  git commit             │  (atomic recovery)
              └─────────────────────────┘
```

## OpBundle Format

An OpBundle is a JSON object describing typed AST operations:

```json
{
  "id": "uuid",
  "originator": "evofabric-agent",
  "target_file": "crates/my-crate/src/lib.rs",
  "ops": [
    {
      "op": "AddAttribute",
      "scope": { "kind": "Function", "name": "my_fn" },
      "attribute": "#[inline]"
    }
  ],
  "rationale": "Why this change was made",
  "status": { "state": "Proposed" }
}
```

### Available Operations

| Op | Description | Key Fields |
|----|-------------|------------|
| `ReplaceFnBody` | Replace a function's body | `scope`, `new_body` |
| `InsertAfter` | Insert new item after target | `scope`, `new_item` |
| `Remove` | Remove an item | `scope` |
| `ReplaceItem` | Replace entire item | `scope`, `new_item` |
| `AddAttribute` | Add attribute to item | `scope`, `attribute` |
| `AddUse` | Add use statement | `path` |

### Scope Targeting

Operations target AST nodes by semantic identity, not line numbers:

```json
{"kind": "Function", "name": "foo"}
{"kind": "Function", "name": "bar", "within": {"kind": "ImplBlock", "type_name": "MyType"}}
{"kind": "ImplBlock", "type_name": "MyType"}
{"kind": "TypeDef", "name": "Config"}
{"kind": "File"}
```
