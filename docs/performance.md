# Performance Instrumentation

Arkavo Edge now exposes dedicated telemetry for latency-sensitive paths. These measurements feed both interactive dashboards and future CI gates.

## Router To Diff Rendering
- `DiffView::set_diff` starts a timer and final rendering records a histogram (`arkavo_router_to_diff_ms`).
- Per-frame render durations are emitted via `arkavo_diff_render_ms` and surfaced in the terminal status bar.
- Logs are tagged with the `arkavo.performance` tracing target for quick filtering.
- When latencies exceed the 50 ms budget, `WARN` entries (`diff_render_over_budget`, `router_to_diff_over_budget`) appear, making breaches easy to spot in log streams.

## A2A Round Trips
- `MetricsCollector::record_rpc_latency` records RPC timings and publishes `arkavo_a2a_round_trip_ms` when the `message/send` method completes.
- Every call emits a debug trace with the measured latency in milliseconds.

## Benchmarks
- Run `cargo bench -p arkavo-terminal diff_render` to profile the diff renderer against a synthetic workload.
- Criterion captures the distribution, enabling regression detection when paired with CI artifact comparison.

## Binary Size Guard
- Execute `cargo xtask check-binary-size --limit-mb 60 --package arkavo` to build the release binary and validate it against the 60 MiB target.
- The command prints the measured size and fails if the threshold is exceeded, allowing straightforward CI wiring.

## Operational Notes
- The new metrics default to no-ops when a recorder is not installed, keeping local runs lightweight.
- Use `RUST_LOG=arkavo.performance=debug` to stream latency logs during exploratory profiling.
- For macOS Metal validation, pair the diff latency traces with `cargo bench` output to confirm GPU acceleration stays within budget.
- The UI telemetry report now exports `last_diff_render_ms` and `last_router_to_diff_ms`, enabling AG-UI or external dashboards to plot recent samples per session.
