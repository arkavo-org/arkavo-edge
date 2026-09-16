# Routine learning pilot

Arkavo can learn checked tool sequences from execution evidence and reuse them
without asking a model to plan each primitive action. This implements a bounded
PANDO-inspired experiment in `arkavo-routines`; it does not adopt PANDO's runtime
or claim its benchmark results.

## Enable the pilot

Build a debug binary with the opt-in feature:

```sh
cargo build -p arkavo --features routines
```

The normal conductor path registers `routine_catalog`, `routine_learn`, and
`routine_run` when a learning bus is available. Specialized roles must explicitly
grant all three meta-tools as well as every primitive they may use. Compiling the
feature never adds grants to a role. The feature also enables session egress
enforcement. Default builds do not expose these tools.

The initial integration covers the conductor's single-task path, including its
parallel planner/executor/judge loop. Separately decomposed complex subtasks and
the standalone chat execution path are not enrolled in this pilot.

## Learning and replay

The LLM proposes a routine through `routine_learn` using actual observation IDs
from `routine_catalog`. Each routine contains 2–8 ordered steps. Each step has an
exact registered tool name, an argument template, and a completion check consisting
of a JSON pointer into the tool result and an expected template value.

Use an observation step to establish environmental preconditions before actions,
and a final observation to check the resulting state. A tool returning success
alone is weaker evidence than checking the requested state change. The engine
checks the supplied predicates; it does not independently prove that the LLM's
predicates capture the user's full intent.

Every string argument or expected string uses a structural input binding such as
`{"$input":"selector"}`. The `inputs` object supplies current values, including
URLs, selectors, paths, scripts, and expected text. String substitution never
interpolates executable source. Fixed booleans and numbers are supported. There
are no stored natural-language instruction bodies or cross-step result bindings.
Parameter names and JSON field names should describe fields, not contain user data.

Admission requires exact agreement with contiguous successful calls in the current
task, including argument values and completion checks. Overlapping calls cannot
serve as an ordered demonstration. Failed calls invalidate the demonstration.
Two different task sessions must demonstrate the same template before it becomes
active. Reusing the same observation IDs cannot manufacture additional evidence.

Catalog retrieval uses exact primitive-tool filtering and deterministic four-entry
pages. Tool definitions remain stable as the library grows; templates arrive as
tool-result data rather than being appended to system instructions.

Replay validates all input bindings and tool availability before starting. Each
primitive then passes through current role grants, the session egress guard and
the original registered tool implementation. Results update session taint before
the next primitive. Calls execute in order; ordinary calls in the same task cannot
interleave with a running routine. Primitive calls have a 30-second timeout and
are reported to learning events and tool timing telemetry.

A failed call or failed predicate stops the routine immediately. Earlier side
effects are not rolled back. Inspect the actual state before retrying. Two
consecutive failures or confidence below 0.5 retire that content version.
Cancellation retires it immediately. Renaming input parameters does not evade the
retired-version record. A materially revised template requires fresh demonstrations.

## Persistence and trust boundaries

When learning persistence is initialized, the library lives beside the learning
database in `routines-<scope digest>.json`. The digest includes agent and swarm
identities. Writes use a private temporary file, synchronization, and atomic
replacement; Unix permissions are 0600. An OS file lock prevents two processes
from independently overwriting one agent's lifecycle state.

Snapshots include bounded templates, evidence-session identifiers, and lifecycle
counters. Raw arguments and results remain in the task's bounded in-memory evidence
window, not the routine snapshot. Ordinary learning-event storage retains its
existing data-handling behavior. Routine snapshots are not a replacement for
existing storage security or DLP policy.

A replay is marked in flight durably before its first primitive. An interrupted
replay is retired on reopening the library. Corrupt or inaccessible storage
disables this pilot for the agent; it does not silently start a fresh library and
forget rejection records. A host that never initializes persistence has an
in-memory library lasting only for that learning bus.

The library is capped at 128 versions, including retired versions. New versions are
refused at capacity. Templates are local to the agent and swarm; there is no gossip
import or automatic conversion of document instructions into policy. Learning a
sequence confers no additional authorization.

## Evaluate adoption

Run the same representative tasks with the pilot disabled and enabled, using the
same model versions, inference budgets, initial application state, and at least
two matched task orders. Include cold starts, warm libraries, and changed layouts
or schemas. Keep test verdicts outside the learning context. Reset application
state between arms and use an independent library per experimental stream.

The bundled `browser_cdp` currently launches a browser for each call. It is not a
persistent browser-session backend for a multi-call browser experiment. Use a
registered MCP backend that preserves a browser session when evaluating browser
routines. The implementation tests use real isolated filesystem operations to
validate replay and policy enforcement; they are not browser success-rate or
LLM token-savings benchmarks.

Export one JSONL measurement per task from the evaluation harness. All fields are
required:

| Field | Meaning |
| --- | --- |
| `task_id`, `phase`, `order_seed` | Matched task identity; phase is `cold`, `warm`, or `changed_environment` |
| `success` | Independent terminal task verdict |
| `execution_tokens` | Prompt, completion and reasoning tokens for execution/planning |
| `learning_tokens` | All routine discovery, catalog and admission inference tokens |
| `verification_tokens` | All model-based reflection and verification tokens |
| `prompt_tokens`, `cached_prompt_tokens` | Prompt totals across all calls; cached tokens remain included in total tokens |
| `repeated_actions`, `primitive_calls` | Count primitive actions, including routine expansion |
| `latency_ms` | Whole-task elapsed time, including learning and verification |
| `max_routing_us` | Highest measured routing latency within the task |
| `policy_violations` | Unauthorized executions detected by the harness; safe denials are not violations |

The three token categories must be disjoint and include unsuccessful attempts and
retries. The runtime's routine catalog reports execution counters, not model token
usage; use actual provider usage or local tokenization from the surrounding
harness. The evaluator consumes measurements; it does not generate them.

```sh
cargo run -p arkavo-routines --bin routine-evaluate -- baseline.jsonl candidate.jsonl
```

The command prints a JSON report and exits with code 2 if the pilot gate fails.
It refuses unmatched or duplicate task identities and inconsistent token counts.
The gate requires at least 20% lower total tokens per successful task, no success
count regression within any phase, no policy violations, and routing within 50 ms.
Cold, warm and changed-environment phases and at least two task orders are required.
This is a screening gate, not a statistical significance test. Use sufficient
independent tasks and repeated runs before deciding to enable the feature by default.

No representative browser/model savings result is established by the code tests.
The default remains off until that experiment passes.

## Validation

```sh
cargo test -p arkavo-routines
cargo test -p arkavo-server --features routines --lib
cargo clippy -p arkavo-routines --all-targets -- -D warnings
cargo clippy -p arkavo-server --features routines --lib -- -D warnings
cargo fmt --all -- --check
```

Tests cover actual filesystem replay, fresh parameter bindings, stale preconditions,
revoked grants, egress denial, failed observations, interrupted execution,
retirement persistence, duplicate evidence, bounded storage, MCP interfaces, and
matched-workload cost accounting.
