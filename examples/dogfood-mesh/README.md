# Dogfood Learning Mesh

Self-improving codebase loop: specialized agents scan Arkavo Edge crates,
write tests, and accumulate improvements on a git branch overnight.
Produces a PR for morning human review.

## How It Works

```
Every 5 minutes:
  1. scan_crate.sh scans a crate (rotate through crates/)
  2. Report feeds into code-reviewer OR test-writer
     (Thompson Sampling picks based on category priors)
  3. Validation is mechanical:
     - Does it compile?  (cargo check)
     - Do tests pass?    (cargo test)
     - Is it clean?      (cargo clippy -D warnings)
  4. Lessons extracted, guidance injected for next cycle
  5. Quality scores recorded for sparklines
```

## Agents

| Agent | Port | Purpose |
|-------|------|---------|
| orchestrator | 8420 | Routes tasks via Thompson Sampling, judges quality, extracts lessons |
| code-reviewer | 8422 | Static analysis, identifies warnings/gaps/issues as structured JSON |
| test-writer | 8424 | Writes unit tests targeting identified gaps |

## Prerequisites

```bash
# Build from repo root
cargo build

# Verify model is available
ls ~/.cache/huggingface/hub/models--unsloth--GLM-4.7-Flash-GGUF

# If missing:
cargo run -p arkavo -- model download glm

# jq is required for JSON processing
brew install jq  # macOS
```

## Quick Start

```bash
cd examples/dogfood-mesh

# Full overnight run: start agents, run loop, create PR
./launch.sh

# Or step by step:
./launch.sh start    # Start agents only
./launch.sh status   # Verify agents are healthy
./launch.sh run      # Run the task loop
./launch.sh pr       # Create PR when done
./launch.sh stop     # Stop all agents
```

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `MAX_CYCLES` | 50 | Number of scan/validate cycles |
| `CYCLE_INTERVAL` | 300 | Seconds between cycles (5 min) |
| `BINARY` | target/debug/arkavo | Path to arkavo binary |

Example overnight run:
```bash
MAX_CYCLES=100 CYCLE_INTERVAL=300 ./launch.sh
```

## Crate Rotation

Targets small, isolated crates (core routing/server/CLI excluded):

1. `arkavo-test-macros` (90 lines)
2. `arkavo-validation` (705 lines)
3. `arkavo-config-encryption` (718 lines)
4. `arkavo-events` (1041 lines)
5. `arkavo-bench` (1275 lines)
6. `arkavo-sbe` (2690 lines)
7. `arkavo-observability` (2711 lines)
8. `arkavo-budget` (2720 lines)

## Monitoring

Watch the learning loop:
```bash
tail -f logs/orchestrator.log | grep -E 'Lesson extracted|Injecting.*guidance|quality='
```

Start the AG-UI to see the Connectome panel:
```bash
cargo run -p arkavo -- ui 7700
# Open http://localhost:7700
```

## Safety

- All work on `dogfood/YYYY-MM-DD` branch, never main
- Every change validated: `cargo fmt --check && cargo clippy -D warnings && cargo test`
- Failed validations revert immediately
- Phase 1 is additive only (new tests) — no existing code modification
- Nothing merges without human review

## What You See in the Morning

- A PR with accumulated test improvements
- The Learning panel shows quality sparklines per agent
- The Connectome shows which agent the mesh trusts for which task type
- Logs tell the full story of what worked and what failed

## Troubleshooting

**Agents show "initializing" in status check:**
Wait 5 more seconds for mDNS discovery, then retry `./launch.sh status`.

**No lessons being extracted:**
Quality may be above 0.5 (acceptable). Check `logs/validate.log` for details.

**Port already in use:**
```bash
lsof -i :8420 -i :8422 -i :8424
./stop.sh
```

**Scan fails for a crate:**
Check `logs/scan.log`. Some crates may have build dependencies not satisfied.
The loop skips failed scans and continues to the next crate.
