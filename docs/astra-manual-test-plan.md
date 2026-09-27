# Astra branch manual test plan

Hands-on verification of `feature/gpt-6-astra` (PR #688, which includes the trust-layer PR #682): GPT-6 Astra support, the local-first harness, cloud consent and budgeting, the tool loop, memory, the merged trust layer, and the regression surface those changes touch.

- Branch: `feature/gpt-6-astra`
- Build: debug only, `cargo build -q`; binary at `target/debug/arkavo`
- Tester needs: a TTY, an OpenAI key, cached local weights, about two hours

## Setup and conventions

**Keys.** `.env` at the repo root holds `OPENAI_API_KEY` and `XAI_API_KEY`. Load with `set -a; source .env; set +a`. Never paste a key into a report.

**Local weights.** `arkavo model list` must show at least one ✓ GGUF (Qwen3.5-0.8B or Gemma 4). To simulate a machine with no weights, prefix a command with `HF_HOME=$(mktemp -d)`.

**Cloud policy.** Set in the YAML frontmatter of `AGENTS.md` (or `.arkavo/AGENTS.md`) in the working directory. Omitted means `ask_before_cloud`.

```yaml
---
budget:
  cloud_policy: local_only      # local_only | ask_before_cloud | cloud_within_cap
  max_cost_per_session: 0.50
---
```

**Consent.** An explicit `--model gpt-6-astra` and an `AGENTS.md` `model:` hint both count as consent. Auto-selection never picks a cloud arm while a local arm is feasible.

**Debug.** `ARKAVO_DEBUG=1 ARKAVO_DEBUG_CHAT=1` prints the model chosen, tool calls with their ids, and the `[Perf]` token line.

**Closed stdin.** Append `</dev/null` to any command that must prove it does not block on a prompt.

**Tags.** `local` runs with no key. `cloud` spends money. `after fix: Task N` means the expected behaviour lands with that follow-up task; until it lands, record the current behaviour and mark the case blocked.

**Recording.** Copy the results table at the end of this document and fill in Pass / Fail / Blocked plus a note (build SHA, observed output, ticket).

## Local-first startup

Commit 00e83291 made local inference mandatory for harness commands. Cloud keys no longer replace weights.

### S-01 Harness refuses to start without local weights, even with a cloud key (local)

Setup: key loaded; empty model cache via `HF_HOME`.

```bash
HF_HOME=$(mktemp -d) arkavo chat --model gpt-6-astra --prompt "hi" </dev/null
HF_HOME=$(mktemp -d) arkavo task 'say hi' --local-only </dev/null
HF_HOME=$(mktemp -d) arkavo ui </dev/null
HF_HOME=$(mktemp -d) arkavo agent </dev/null
```

Expect: each exits non-zero with "The agent harness requires local models. Provision them with `arkavo model download`…". No download starts and no request reaches OpenAI (set `OPENAI_API_KEY=sk-invalid` and confirm no 401 is logged).

### S-02 Utility commands run without weights (local)

```bash
HF_HOME=$(mktemp -d) arkavo --help
HF_HOME=$(mktemp -d) arkavo model list
HF_HOME=$(mktemp -d) arkavo agent init smoke-agent   # in a scratch dir
HF_HOME=$(mktemp -d) arkavo mcp proxy --help
```

Expect: all succeed with no first-run prompt and no model download.

### S-03 Interactive first run offers the sized Gemma download (local)

Setup: real terminal, empty `HF_HOME`, network available.

1. `HF_HOME=$(mktemp -d) arkavo chat`
2. Decline the prompt and note the exit.
3. Run again and accept.

Expect: decline gives a clean error with no hang. Accept downloads Gemma 4 sized to this device (E2B plus 12B on a desktop), then chat starts on the local model.

### S-04 ARKAVO_SKIP_FIRST_RUN skips the prompt and never downloads (local, after fix: Task 6)

```bash
ARKAVO_SKIP_FIRST_RUN=1 arkavo chat --prompt "hi" </dev/null                       # weights present
HF_HOME=$(mktemp -d) ARKAVO_SKIP_FIRST_RUN=1 arkavo chat --prompt "hi" </dev/null  # weights absent
```

Expect: with weights present it proceeds on the local model with no prompt. With weights absent it gives the S-01 error and downloads nothing. Today both branches error, and the Dockerfile and self-host docs still rely on the variable.

### S-05 Default chat stays local with a key present (local)

```bash
ARKAVO_DEBUG=1 arkavo chat --repo-context off --prompt "Name three primary colours." </dev/null
```

Expect: the `[Model]` line names a local model (Gemma 4 or Qwen). No cloud request and no consent prompt. Repeat with `OPENAI_API_KEY` unset: same result.

## Astra provider

The OpenAI Responses provider: text, native tools with call-id continuity, strict schema, streaming, usage.

### A-01 One-shot text reply (cloud)

```bash
ARKAVO_DEBUG=1 arkavo chat --model gpt-6-astra --repo-context off --prompt "Reply with the single word ready." </dev/null
```

Expect: the local model loads first (local-first), then `[Model] gpt-6-astra (--model override)`, a sensible reply, and a `[Perf]` line with prompt and generation token counts above zero. Known: the millisecond figures read 0 and throughput shows "? tok/s" until Task 7 lands.

### A-02 Hello-world example on Astra (cloud)

```bash
cd examples/01-hello-world && ./run.sh --model gpt-6-astra </dev/null
```

Expect: an introduction reply within about ten seconds of the model line. Passed on 2026-09-07; re-run after each follow-up commit.

### A-03 Native tool call is executed and paired by call id (cloud)

```bash
ARKAVO_DEBUG=1 ARKAVO_DEBUG_CHAT=1 arkavo chat --model gpt-6-astra --repo-context off \
  --prompt "Use the get_agent_time tool, then tell me the current time." </dev/null
```

Expect: debug shows a tool call with an id of the form `call_…`, the tool executing, and a tool-role result carrying the same id. The final answer states a time consistent with the tool output. No API error about a missing `function_call_output`.

### A-04 Multi-turn continuity keeps reasoning state and tool history (cloud)

1. `ARKAVO_DEBUG_CHAT=1 arkavo chat --model gpt-6-astra --repo-context off`
2. Turn 1: "Use get_agent_time and tell me the time."
3. Turn 2: "What exactly did that tool return, verbatim?"
4. Turn 3: "Now list the models available with list_models."
5. `/history`, then `/exit`.

Expect: turn 2 quotes turn 1's tool output. Every turn succeeds with HTTP 200. A 400 mentioning `function_call`, `reasoning`, or an unknown item id means replay of provider state broke. History shows the tool turns in order.

### A-05 Streaming shows tokens incrementally and cancels cleanly (cloud)

1. Interactive `arkavo chat --model gpt-6-astra --repo-context off`.
2. Ask for a 400-word story and watch the output.
3. Ask again and press Ctrl-C mid-stream.

Expect: words arrive progressively rather than all at once. Ctrl-C returns to the prompt (or exits) within a second and no stray text is printed afterwards.

### A-06 Fixture and live provider suites (cloud)

```bash
cargo test -q -p arkavo-llm --test openai_responses
cargo test -q -p arkavo-llm --test openai_responses live_astra -- --ignored --nocapture
```

Expect: nine deterministic fixture tests pass; the live test passes (text, tool call, call-id continuation, strict schema, stream) in under a minute. The live run charges five small requests.

### A-07 Invalid key fails fast and legibly (cloud, after fix: Task 7)

```bash
OPENAI_API_KEY=sk-invalid arkavo chat --model gpt-6-astra --repo-context off --prompt "hi" </dev/null
```

Expect: one error naming HTTP 401, no panic, no retry storm, no key text in the output. After Task 7 the error also carries the API's `error.code` (for example `invalid_api_key`).

### A-08 Oversized tool output is bounded without a crash (cloud)

```bash
ARKAVO_DEBUG_CHAT=1 arkavo chat --model gpt-6-astra --repo-context off \
  --prompt "Run shell_exec with: python3 -c \"print('日本語😀' * 60000)\" and then say how long the output was." </dev/null
```

Expect: no panic. Debug shows the tool result truncated at a character boundary with a truncation marker, the same call id retained, and the model acknowledges the output was cut.

## Cloud policy and consent

Policy is enforced before any provider is built. Consent comes from an explicit model, a manifest hint, or an interactive answer scoped to one session.

### P-01 local_only refuses an explicit cloud model before any request (local)

Setup: scratch dir with an `AGENTS.md` whose frontmatter sets `cloud_policy: local_only`; export `OPENAI_API_KEY=sk-invalid`.

```bash
ARKAVO_DEBUG=1 arkavo chat --model gpt-6-astra --repo-context off --prompt "hi" </dev/null
```

Expect: a policy error naming LocalOnly. No 401 appears, proving the refusal happened before the network. The same command with `--model ministral-3b` works.

### P-02 Manifest model hint counts as consent (cloud)

Setup: `AGENTS.md` with an agent section whose `model: gpt-6-astra`; no `cloud_policy` (default ask_before_cloud).

```bash
ARKAVO_DEBUG=1 arkavo chat --repo-context off --prompt "Reply with ready." </dev/null
```

Expect: Astra answers with no consent prompt (documented behaviour: a manifest hint is explicit consent). Remove the hint: the same command stays local.

### P-03 Non-interactive client never hangs on a consent prompt (cloud, after fix: Task 2)

Setup: terminal 1 runs `arkavo agent -v -p 8343` in a dir with default policy and no model hint; terminal 2 is a client.

```bash
# terminal 2
arkavo chat --agent-id <agent id from terminal 1> --prompt "Use gpt-6-astra to reply with ready." </dev/null
```

Then, from terminal 2, request something that requires cloud through the agent's chat session (a model hint in the request, or an `AGENTS.md` for the client that names gpt-6-astra).

Expect: the remote client receives a clear "cloud confirmation required" policy error immediately. Current bug: the server process shows the yes/no prompt on its own terminal and the client waits until someone answers it there.

### P-04 Interactive consent is asked once per session and scoped to it (cloud, after fix: Task 2)

Setup: interactive `arkavo chat` on a TTY, default policy, key loaded, and a path that needs cloud (an `AGENTS.md` without a model hint but with `cloud_policy: ask_before_cloud`, and a request that only a cloud arm satisfies).

1. Session A: trigger cloud, answer `y`. Trigger again.
2. Session B (second terminal, same agent or a fresh `arkavo chat`): trigger cloud.
3. Session C: answer `n`, then trigger again.

Expect: A asks once and not again. B is asked independently. C refuses and does not re-ask within the session. Nothing about consent persists across process restarts. Current bug: one "y" authorises every session served by the same process.

### P-05 cloud_within_cap refuses when the cap cannot cover the request (local)

Setup: `AGENTS.md` with `cloud_policy: cloud_within_cap` and `max_cost_per_session: 0.0001`; `OPENAI_API_KEY=sk-invalid`.

```bash
ARKAVO_DEBUG=1 arkavo chat --model gpt-6-astra --repo-context off --prompt "Write 300 words." </dev/null
```

Expect: a budget-exceeded error before dispatch; no 401 in the log.

### P-06 Auto-selection never picks cloud while a local arm is feasible (local)

With the key loaded and no hint, run ten varied prompts through `ARKAVO_DEBUG=1 arkavo chat --prompt …` (code, a summary, a long context, a question needing tools).

Expect: every `[Model]` line is local. No consent prompt appears.

## Budget and usage

Every Astra call records cached input, visible output and reasoning once each; fractional cents accumulate.

### B-01 Small calls accumulate instead of rounding to zero (cloud)

Setup: `AGENTS.md` with `cloud_policy: cloud_within_cap` and `max_cost_per_session: 0.02`.

In one interactive Astra session send "ok" twenty times, then ask for 800 words.

Expect: the long request is eventually refused with budget-exceeded once accumulated spend passes two cents, and the refusal happens before an HTTP request (check with debug). Spend records in the log name provider `openai` and model `gpt-6-astra`.

### B-02 Usage counts reconcile with the OpenAI dashboard (cloud)

Note the dashboard's usage for the key, run A-01 and A-03, note the deltas, and compare with the `[Perf]` token counts.

Expect: prompt tokens match input tokens; the logged generation count plus reasoning equals the dashboard's output tokens (reasoning is not double counted).

### B-03 Two agents cannot both spend the last cent (cloud, after fix: Task 10)

One agent with `max_cost_per_session: 0.01`; from two clients send a 600-word request at the same moment.

Expect: exactly one dispatches, the other gets budget-exceeded. Today both can pass the check (non-atomic can_afford).

## Tool loop and sessions

### T-01 Local model tool loop unchanged (local)

```bash
ARKAVO_DEBUG_CHAT=1 arkavo chat --repo-context off --prompt "Use get_agent_time and tell me the time." </dev/null
```

Repeat with `--model ministral-3b` and with a Gemma 4 model.

Expect: the tool call is parsed from the model's text format, executed, and its result paired (id `call_0` style); the answer contains the time. Tool results are sent with the tool role on providers that support it.

### T-02 Agent conductor loop with parallel tools (local)

Setup: terminal 1 runs `arkavo agent -v -p 8343` from `examples/01-hello-world`.

```bash
arkavo chat --agent-id hello-agent --prompt "Get the agent time and list the available models, then summarise both." </dev/null
```

Expect: both tools run; results are attributed to the right calls in the summary; the session ends without the client hanging.

### T-03 Huge tool output in the conductor path (local, after fix: Task 5)

Through the agent from T-02: "Run shell_exec with `python3 -c "print('é'*300000)"` and tell me the first character."

Expect: no panic in the agent terminal, a bounded result, answer "é". Three byte-slice sites in the server loop can still panic on multi-byte text at the cut; Task 5 fixes them.

### T-04 Terminal session restores prior turns (local, after fix: Task 5)

1. `arkavo terminal`, ask two questions, exit.
2. Run `arkavo terminal` again and ask "What did I ask you first?".

Expect: "Restored previous conversation" is printed and the answer references the first question. Persistence of tool turns and provider state is unwired until Task 5.

### T-05 Closing a session delivers a stream end, not silence (local)

With the agent from T-02 running, start an interactive `arkavo chat --agent-id hello-agent`, ask for a long answer, and stop the agent process (Ctrl-C in terminal 1) mid-stream.

Expect: the client shows a terminated-stream error within a few seconds and returns to its prompt rather than waiting forever.

## Memory

### M-01 Default build search is deterministic and complete (local)

In `arkavo terminal` (MCP memory tools enabled), store five memories, then call the memory search tool twice with the same query and a limit of 10.

Expect: both calls return all five in the same newest-first order. Restart the process and repeat: same order.

### M-02 Embeddings build: placeholders do not outrank real matches (local, after fix: Task 9)

```bash
cargo build -q -p arkavo --features embeddings
```

Store three topical memories, hold a short conversation (which stores zero-vector placeholders), then search for one topic.

Expect: the topical memory ranks first; conversation placeholders are absent or last. Today every placeholder ties at distance 0 and ranks first.

### M-03 Existing memory database opens after upgrade (local)

Use a memory database created on main (copy one aside first), run the branch binary, list and search.

Expect: it opens, migrates in place, and searches return the old rows; no decode error on embedding blobs.

## Trust layer

Merged from PR #682. Only `arkavo mcp proxy` is permit-gated; in-process tool calls and cloud dispatch are not.

### X-01 MCP proxy refuses tools/call without a valid permit (local)

Setup: Python 3 available for the echo fixture.

```bash
arkavo mcp proxy --policy-bundle-hash $(printf '%064d' 0) --issuer-key <hex issuer key from docs/dispatch-gate.md> \
  -- python3 crates/arkavo-mcp-proxy/tests/fixtures/echo_mcp_server.py
```

On stdin send `initialize`, `tools/list`, then a `tools/call` with no `_meta.permit`, then one with a garbage permit.

Expect: `initialize` and `tools/list` pass through; both calls get a JSON-RPC error refusal and never reach the echo server; the proxy exits cleanly on EOF.

### X-02 Delegation JWTs are ignored unless a verification key is configured (local)

1. Start `arkavo agent -v` with `ARKAVO_DELEGATION_PUBLIC_KEY_PEM` unset; register a peer presenting a delegation JWT.
2. Repeat with `ARKAVO_ALLOW_UNVERIFIED_DELEGATION=1`.
3. Repeat with the PEM set to a wrong key.

Expect: step 1 registers with no delegated entitlements. Step 2 grants entitlements with an insecure-mode warning. Step 3 refuses entitlements (signature mismatch). The hatch cannot override a configured key.

### X-03 Gateway binds loopback by default and answers probes (local)

```bash
arkavo ui 7700 &
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:7700/healthz
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:7700/readyz
curl --connect-timeout 2 http://<this machine's LAN ip>:7700/healthz   # from another host
ARKAVO_AGUI_BIND=0.0.0.0 arkavo ui 7701 &
ARKAVO_AGUI_BIND=not-an-ip arkavo ui 7702 &
```

Expect: on 7700, healthz returns 200 at once, readyz returns 200 once the model is loaded, and the LAN connection is refused. 7701 is reachable from the LAN. 7702 logs a warning and binds loopback.

### X-04 KAS trusted roots deny when absent or inconsistent (local)

Run `cargo test -q -p arkavo-server --features kas --test kas_delegation_test`; then start an agent whose `kas:` block lists a root whose `public_key` does not match its DID.

Expect: the tests pass. The mismatched root is dropped with a warning at startup and delegation verification reports no trusted root.

## Regression areas

Changes that touched shared code: tool-role rendering for every provider, provider feature forwarding, streaming, the architect and UI planners, deployment docs.

### R-01 Other cloud providers still complete a tool loop (cloud)

Setup: keys for whichever of Anthropic, Gemini, DeepSeek, Kimi, xAI are available; model names from `arkavo model list`.

For each: `ARKAVO_DEBUG_CHAT=1 arkavo chat --model <name> --repo-context off --prompt "Use get_agent_time and tell me the time."`

Expect: each completes; the tool result is rendered as user text for Anthropic, DeepSeek, Kimi and Gemini, never as an assistant prefill. Anthropic currently loses the assistant's tool_calls in the replayed turn; Task 8 renders proper tool_use blocks.

### R-02 Ollama path does not leak provider state on the wire (local)

Setup: Ollama running with any model.

`arkavo terminal` (picks Ollama), run a tool question, then inspect Ollama's request log or a local proxy capture.

Expect: requests contain role, content and tool fields only; no `provider_state` object.

### R-03 Architect mode plans on Astra and executes locally (cloud, after fix: Tasks 3 and 4)

Setup: a scratch repo; `AGENTS.md` with `model: gpt-6-astra`.

```bash
ARKAVO_DEBUG=1 arkavo task 'add a README with an install section and a usage section, and a CONTRIBUTING file' --local-only
```

Expect: planning uses Astra; subtasks run on the cached local model (local-first); the summary's cost line is consistent with the recorded spend. The "saved vs Opus" figure is a hardcoded comparison until Task 4, and escalation may pull an uncached model until Task 3.

### R-04 UI generator with Gemini still produces a page (cloud, after fix: Task 4)

```bash
GEMINI_API_KEY=… arkavo ui --prompt "Build a two-column settings page"
```

Expect: the page renders in the browser; under `cloud_policy: local_only` the planner refuses cleanly instead of calling Gemini. Until Task 4 the UI planner obtains a raw Gemini client outside the policy gate.

### R-05 Cloud-only and Windows feature sets build without llama.cpp (local)

```bash
cargo build -q -p arkavo --no-default-features --features memory,mdns,mcp-tools,llm-remote,web-ui
cargo tree -p arkavo --no-default-features --features windows-default -e normal | grep -ci llama
```

Expect: it builds and the grep prints 0. The resulting binary runs `arkavo --help` and `arkavo model list`, and refuses `arkavo chat` with the local-models error.

### R-06 Container docs match the binary's behaviour (local, after fix: Task 6)

Follow `docs/deploy/container.md` to build the slim image, then follow the compose example in `docs/deploy/self-host.md`.

Expect: utility commands work in the container; the documented `ui` deployment starts with a mounted model cache. Today the compose and Kubernetes examples run the utility image with only a cloud key and cannot start.

### R-07 Security suites (local)

```bash
cargo test -q -p arkavo-protocol --test security_vulnerabilities
cargo test -q -p arkavo-cli --lib mock_provider::
./tests/e2e_security_test.sh
./tests/security_cli_test.sh
./tests/dlp_pii_security_test.sh
```

Expect: all pass. Known accepted skips: one pre-existing signature-timing test ignored; two binary-inspection checks skipped in the CLI script.

### R-08 Mesh discovery and A2A chat unaffected (local)

Start two agents from two example dirs with `mdns: true`; from a third terminal run `arkavo chat --agent-id <first>` and `arkavo task 'say hello' --mesh-only`.

Expect: the agents discover each other; chat and task both reach an agent; sessions close cleanly.

### R-09 Router unit suite passes on a machine with cached weights (local)

```bash
cargo nextest run -p arkavo-router
```

Expect: all pass with the real HuggingFace cache (fixed in Task 1, commit af2f24b0). Before that fix three architect tests failed on any dev box with Qwen 0.8B cached.

## Follow-ups in flight

Ten tasks are being landed on the branch. Re-run the cases named here once each commit appears in `git log`.

| Task | Change | Re-test |
|---|---|---|
| 1 | Router tests off the host cache; gate_latency bench compiles; CI runs orchestrator and KAS tests | R-09, X-04 |
| 2 | Cloud consent asked through the host, scoped per session; no stdin in the protocol crate; agent regains approval path | P-03, P-04, T-02, T-05 |
| 3 | Gate before provider construction on every path; no on-demand weight download during routing | P-01, P-05, R-03, S-01 |
| 4 | Ungated planning provider removed; savings summary and dead budget middleware removed; local-first task planning | R-03, R-04 |
| 5 | One tool-result pairing and bounding helper; UTF-8 panics fixed; terminal persistence wired | A-08, T-01, T-03, T-04 |
| 6 | First-run skip defined under local-first; deployment docs and manifests corrected | S-04, R-05, R-06 |
| 7 | Astra error codes surfaced; effort-scaled timeout; unknown items tolerated; timing populated; streamed state kept | A-01, A-04, A-05, A-07 |
| 8 | Shared SSE decoder; Anthropic tool_use blocks; Debug cleanups | A-05, R-01 |
| 9 | Zero-norm embeddings kept out of the index; strict blob decoding | M-01, M-02, M-03 |
| 10 | One pricing table; reserve-then-settle budgeting | B-01, B-02, B-03, P-05 |

## Results

| ID | Title | Result | Note |
|---|---|---|---|
| S-01 | Harness refuses to start without local weights | | |
| S-02 | Utility commands run without weights | | |
| S-03 | Interactive first run offers the sized Gemma download | | |
| S-04 | ARKAVO_SKIP_FIRST_RUN skips the prompt, never downloads | | |
| S-05 | Default chat stays local with a key present | | |
| A-01 | One-shot text reply | | |
| A-02 | Hello-world example on Astra | | |
| A-03 | Native tool call is executed and paired by call id | | |
| A-04 | Multi-turn continuity keeps reasoning state and tool history | | |
| A-05 | Streaming shows tokens incrementally and cancels cleanly | | |
| A-06 | Fixture and live provider suites | | |
| A-07 | Invalid key fails fast and legibly | | |
| A-08 | Oversized tool output is bounded without a crash | | |
| P-01 | local_only refuses an explicit cloud model before any request | | |
| P-02 | Manifest model hint counts as consent | | |
| P-03 | Non-interactive client never hangs on a consent prompt | | |
| P-04 | Interactive consent is asked once per session and scoped to it | | |
| P-05 | cloud_within_cap refuses when the cap cannot cover the request | | |
| P-06 | Auto-selection never picks cloud while a local arm is feasible | | |
| B-01 | Small calls accumulate instead of rounding to zero | | |
| B-02 | Usage counts reconcile with the OpenAI dashboard | | |
| B-03 | Two agents cannot both spend the last cent | | |
| T-01 | Local model tool loop unchanged | | |
| T-02 | Agent conductor loop with parallel tools | | |
| T-03 | Huge tool output in the conductor path | | |
| T-04 | Terminal session restores prior turns | | |
| T-05 | Closing a session delivers a stream end, not silence | | |
| M-01 | Default build search is deterministic and complete | | |
| M-02 | Embeddings build: placeholders do not outrank real matches | | |
| M-03 | Existing memory database opens after upgrade | | |
| X-01 | MCP proxy refuses tools/call without a valid permit | | |
| X-02 | Delegation JWTs are ignored unless a verification key is configured | | |
| X-03 | Gateway binds loopback by default and answers probes | | |
| X-04 | KAS trusted roots deny when absent or inconsistent | | |
| R-01 | Other cloud providers still complete a tool loop | | |
| R-02 | Ollama path does not leak provider state on the wire | | |
| R-03 | Architect mode plans on Astra and executes locally | | |
| R-04 | UI generator with Gemini still produces a page | | |
| R-05 | Cloud-only and Windows feature sets build without llama.cpp | | |
| R-06 | Container docs match the binary's behaviour | | |
| R-07 | Security suites | | |
| R-08 | Mesh discovery and A2A chat unaffected | | |
| R-09 | Router unit suite passes on a machine with cached weights | | |
