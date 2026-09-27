# Shadow Teacher Kit

A small data-gathering swarm for the shadow consolidation teacher. Two
coordination-plane workers execute tool-driven tasks with local models,
producing real episode traces in the learning store; the audit-plane
`shadow-teacher` role in the kit manifest demonstrates the plane
separation rule (no handoffs, no context writes — the teacher observes,
it never drives work). The `arkavo-shadow-consolidation` job then reads
the gathered episodes read-only and previews the consolidation batch
with a zero-spend dry run.

## What it demonstrates

- **Plane declaration**: the kit manifest carries `plane: "audit"` on the
  shadow-teacher role; kit validation rejects audit-plane roles with
  handoffs or context writes.
- **Episode gathering**: tasks force the workers to read files with their
  filesystem tools, so tool observations flow through the learning
  pipeline (`observations -> synthesized episodes -> .arkavo/learning/lessons.db`).
- **Shadow consolidation**: the standalone job opens the merged store
  read-only, strips observe/list calls per the lesson-synthesis rule,
  batches each task category, and (in dry-run) writes the exact prompts
  it would send to Fable — plus the cost ledger scaffolding — without
  spending anything.

## Requirements

- `cargo build` from the repo root (debug binary is fine)
- The local model: `hf download ggml-org/gemma-4-E4B-it-GGUF gemma-4-E4B-it-Q4_K_M.gguf`
  (both workers share it; episode synthesis reuses the loaded model)
- `jq`, `curl`, `sqlite3`
- `node`/`npx` — the workers get filesystem tools from
  `@modelcontextprotocol/server-filesystem` (auto-fetched on first run,
  rooted at each agent's `workspace/`)

## Run

```bash
cd examples/shadow-teacher-kit
./run.sh
```

The script validates and launches the kit manifest, starts both agents,
submits the six tasks from `tasks.json` sequentially (local inference
takes minutes per task), waits for episode synthesis, merges the two
agents' learning stores into `out/episodes.db`, and runs:

```bash
arkavo-shadow-consolidation --db out/episodes.db --out out/shadow --min-episodes 2 --dry-run
```

Artifacts land in `out/shadow/`:

- `prompts.json` — the exact per-category prompts Fable would receive
- `cost_ledger.json` — zeroed ledger with batch/episode accounting
- `lessons.json`, `proposals.json`, `findings.json`, `rejects.json` —
  empty in dry-run; populated by a live run

## Live run (optional, real spend)

```bash
export ANTHROPIC_API_KEY=...
cargo run -p arkavo-shadow-consolidation -- \
  --db examples/shadow-teacher-kit/out/episodes.db \
  --out examples/shadow-teacher-kit/out/shadow-live \
  --min-episodes 2 --max-run-cost-usd 1.0
```

The run-level ceiling stops the job between batches and marks the ledger
`budget_exhausted`; partial outputs always survive.

## Stop / cleanup

```bash
./stop.sh                  # stop agents
rm -rf out logs */workspace */.arkavo   # reset gathered data
```
