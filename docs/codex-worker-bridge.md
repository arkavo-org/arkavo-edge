# Codex worker bridge

Arkavo Edge runs a Codex coding worker through Codex's non-interactive JSONL
interface. The integration is carried by the `codex-agent` feature, which is on
by default for the desktop binary (like `claude-agent`, and like it absent from
`windows-default` and `minimal`). It uses the Codex executable already available
on `PATH`; it does not download or authenticate Codex, and everything it adds is
inert when no `codex` executable is found.

## CLI

The command ships in the default binary:

```bash
cargo run -p arkavo -- codex --prompt "Review the parser for error handling"
```

The command accepts `--workspace PATH` (default `.`), `--write` to grant
workspace writes, and `--resume STATE_FILE` to continue the session described
by a state file printed by an earlier invocation. Without `--write`, the
worker uses a read-only sandbox.

`--acknowledge-unrecorded-spend DOLLARS` is the operator's repair for a session
an interrupted attempt left marked for reconciliation. It records that charge,
clears the mark, prints the state file it cleared, and exits without running
anything; a prompt is refused alongside it, because starting work would stack a
second unmeasured attempt on the first. The figure is an assertion about the
provider's bill — nothing in Edge observed it, which is why the session refuses
to run until a person supplies it — and `0` asserts that the attempt cost
nothing rather than assuming it. The token breakdown is left at zero: what was
reconciled is a charge, not a usage report. There is no automatic reconciliation
and the refusal message names this flag. Codex authentication comes from the saved
Codex authentication available to the Codex CLI; Arkavo does not manage those
credentials.

Each session is bound to its workspace, agent identity, model, sandbox, and
Codex thread. The default model is `gpt-6-astra`. A prompt cannot change those
values. Session state is owned by the host and stored under `~/.arkavo/codex`,
named for the identity and workspace exactly as a registered worker's is, so
repeated invocations in one workspace continue one session instead of stranding
a file per run. A read-only run and a `--write` run are separate identities, and
so separate files, because the store refuses a binding whose sandbox changed. A
lock prevents concurrent ownership — a second `arkavo codex` in the same
workspace and grant level is refused while the first is running — and resuming a
state file with a different binding is rejected.

State may not sit inside the worker's workspace, because the worker can be
granted writes there and would then be able to rewrite its own binding. The one
exemption is `~/.arkavo` itself: a host whose workspace is the home directory
(or `/`) contains that tree only because the grant is wide, not because it names
the state directory. The exemption ends as soon as the workspace *is* `~/.arkavo`
or something inside it. The rendezvous that keeps two workers off one Codex
thread lives under `~/.arkavo/codex/threads` rather than in the shared temporary
directory, where any local account could pre-plant a symlink at the well-known
name. Codex authenticates per user, so a thread is owned per user and a private
directory gives up nothing.

## MCP registration

The `arkavo-mcp-codex` library registers `codex_run`, `codex_status`, and
`codex_cancel` through its `register_tools` function. The host must authorize
the worker's complete sandbox and construct the worker before registering these
tools. The MCP arguments carry a prompt or cancellation/status request only;
they cannot grant permissions, change the workspace, select a model, or replace
the session.

Edge registers them at the three registries that already carry capability tools:
the CLI tool loop, `LocalEngine`, and the A2A server. Registration is gated at
runtime, not by configuration — a registry gets the tools only when a `codex`
executable is on `PATH`, the resolved cloud policy is not `local_only`, and the
worker opens its session state successfully. Anything else is logged and
skipped, so an install without Codex behaves exactly as before.

Registered workers always get the read-only sandbox. A tool call is not a
person, so an LLM-reachable worker must not carry a workspace-write grant nobody
issued for that run; `arkavo codex --write` remains the only path to writes.
For the same reason registration passes `user_confirmed: false`. Under the
default `ask_before_cloud` policy `codex_run` therefore refuses at call time
with the spend plane's confirmation verdict, which is how every other cloud path
behaves under that policy.

Codex's shell and file operations run inside its authorized sandbox and do not
pass through Arkavo Edge's per-tool OpenTDF policy hooks. Treat registration as
a host-owned explicit grant of the worker capability and workspace authority.
The sandbox's workspace boundary constrains writes; it is not a per-file read
allowlist. Grant only trusted workspaces, including their Codex configuration,
hooks and MCP integrations. Use external isolation where role confidentiality
requires stronger controls.

The host constructs `CodexConfig`, `SpendApproval`, and `CodexWorker::open`,
passing its shared `Arc<BudgetTracker>` and an external state path. Register the
resulting `Arc<CodexWorker>` with `register_tools`. `arkavo_server::codex` does
this for every Edge entry point: it derives the rate card and admission estimate,
places the session state, and calls `register_tools`, so one rate card and one
state rule serve the CLI loop, `LocalEngine`, the A2A server and the command.

Registered workers use the current directory as the workspace and a session
state file named for the agent identity and a digest of that workspace, under
`~/.arkavo/codex`. The name is stable on purpose: a crash mid-run leaves the
binding marked `accounting_incomplete`, and only a file the next process reopens
can force the host to reconcile that charge. The digest keeps one identity's
sessions in different checkouts apart, because the store rejects a saved binding
whose workspace changed.

One worker — one session, one lock — is kept per identity and workspace for the
life of the process. Registries are rebuilt while the previous one is still
referenced (a new request, an agent-config hot-reload), and the session lock is
exclusive, so opening a worker per registration would make the tools disappear
after the first rebuild.

A worker carries the spend policy it was opened with, so the cached worker is
reused only while the requested policy still matches. When an AGENTS.md reload
changes it, the cache entry is dropped and the session reopened; the file lock
decides whether that succeeds. If anything still holds the previous worker the
reopen fails and the tools are simply not registered, so no run happens under a
superseded posture — and because the stale entry is gone, the next registration
after that reference is released picks up the new policy without a restart.
Several agents launched in the same directory under the same identity share the
one session.

## Invocation contract

Verified against `codex-cli 0.153.4`. The worker invokes:

```
codex exec [resume <thread-id>] --json --ignore-user-config --ignore-rules \
     --skip-git-repo-check --model <model> \
     -c sandbox_mode="read-only|workspace-write" \
     -c approval_policy="never" \
     -c sandbox_workspace_write.network_access=false \
     -c web_search="disabled" \
     -c shell_environment_policy.inherit="none" \
     -
```

`resume` is a clap **subcommand**, not a flag: `codex exec resume [OPTIONS]
[SESSION_ID] [PROMPT]`. It carries its own option set, and that set has no
`--sandbox`. The permission therefore travels as `-c sandbox_mode="…"`, which
both forms accept, and `resume <thread-id>` sits immediately after `exec` with
every option following it. A resumed turn re-emits the same `thread_id`, which
the reader accepts rather than treating as a session switch.

`--skip-git-repo-check` is required because `--ignore-user-config` also unloads
the user's trusted-directory list, reducing Codex's own guard to "is this a git
repository". The host, not that heuristic, is the workspace authority, and the
sandbox still bounds the run; without the flag an ordinary non-git workspace
fails with a message that only reaches the deliberately suppressed stderr. The
flag does not touch the sandbox.

The prompt is always written to stdin and passed as `-`. Each `-c` key is typed
and known to the config loader: a bad *value* on any of them fails bootstrap,
whereas an unknown key is silently ignored, so the keys were confirmed by their
rejection behaviour rather than by acceptance alone.

Two live read-only runs (a fresh `exec` and a `resume` of the same thread) are
recorded as fixtures under `crates/arkavo-mcp-codex/tests/fixtures/` and are
parsed by the crate's own tests. The resumed run refused a write, so
`sandbox_mode="read-only"` is enforced on a resumed thread and not only on a
fresh one.

**Not yet observed:** neither live run produced a `command_execution` item, so
`shell_environment_policy.inherit="none"` and
`sandbox_workspace_write.network_access=false` are validated as accepted, typed
configuration — not as observed behaviour under shell use. A worker whose every
shell command fails for want of an inherited environment (no `PATH`) is a
plausible failure mode that no run has ruled out. `file_change` and
`turn.failed` event shapes are likewise pinned from the binary's serde tables
and upstream source rather than from a transcript, because producing them needs
a workspace-write run.

## Spend estimates and recovery

The worker performs admission using a projected spend estimate and records an
API-price estimate from reported input, cached-input, output, and reasoning
tokens. The estimate is not a hard cap and is not a billing statement. Codex
usage may not reveal actual cache writes, long-context pricing, pricing tiers,
or subscription billing, so the amount can differ from the provider's actual
charge.

Both figures come from the router's price table for `gpt-6-astra` rather than a
second copy of the rate card: the per-MTok rates are read back out of
`ModelChoice::usage_cost_usd`, and the admission estimate is that function
applied to the router's own single-request token profile for a code-generation
task. Every rate is a total for its own bucket rather than a
surcharge over another. Codex reports cached and cache-written tokens inside
`input_tokens`, so the worker makes the three buckets disjoint before pricing
them — exactly as the router's own cost calculation does — and each token is
charged once, at its own rate. A Codex `exec` run is an internal agentic loop rather than one request, so
a long run routinely exceeds the admission figure. What the gate guarantees is
that a run cannot start against an exhausted cap, not that it will stay inside
one; the measured charge is recorded afterwards.

Every entry point spends through the budget tracker its router already uses, so
inference and delegated coding share one ledger. The `arkavo codex` command owns
its own spend plane and builds it from the same AGENTS.md budget block — its
caps, its posture — so the admission estimate is weighed against the configured
cap rather than against a fixed number. That tracker is built fresh for each
invocation and holds no ledger on disk, so what the cap binds is the spend
inside one command run; a second run starts from zero however much the first
spent. Enforcing a daily cap needs what the crate already asks a long-lived
orchestrator for — durable ledger storage across restarts, the same storage
`reconcile` expects a host to deduplicate against. An `AskBeforeCloud`
confirmation is consumed by one attempt; tool arguments cannot renew it.
`LocalOnly` always refuses execution, and the tools are not registered at all
under that policy.

If a run starts without complete usage reconciliation, the state is marked
`accounting_incomplete` and another run is refused until the trusted host
reconciles provider usage and cost through the host API. A missing usage record
is not treated as zero spending. Reconciliation remains incomplete when the
host cannot obtain the provider's authoritative usage or cost.
The host-only `reconcile(usage, dollars)` method records the recovered charge
and clears the incomplete flag. If a crash occurred between recording a charge
and saving state, the host must deduplicate against its ledger first.

Cancellation, timeout, and dropping the run future stop the worker process
tree using a Unix process group or Windows job object. The Codex session ID is
saved as soon as it appears, so interruption preserves the resume binding.

## Library version

`arkavo-mcp-codex` is a self-contained capability crate: it depends only on
`arkavo-budget` for the spend types, plus `arkavo-mcp` and `arkavo-mcp-tools`
behind its `mcp-tools` feature, so a host that wants the worker without the MCP
surface takes neither. It carries `version.workspace = true`, so it releases on
the workspace version alongside the rest of the tree rather than on a cadence of
its own.

## References

- [Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
- [GPT-6 Astra model documentation](https://developers.openai.com/api/docs/models/gpt-6-astra)

This document describes the integration from the implementation, and its
invocation and event contract from two recorded live `codex exec` runs against
`codex-cli 0.153.4`, with the one exception noted under "Invocation contract".
