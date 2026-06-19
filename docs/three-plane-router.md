# Three-Plane Router

llama.cpp statistics measure feasibility and cost. They do **not** measure
answer quality. Therefore they must never silently authorize cloud spend.

The router separates three concerns that were previously conflated. Each plane
answers a different question, and the planes may not borrow each other's
authority.

```text
Availability changes what is possible.   -> feasibility plane
Quality changes what is advisable.        -> collapse / adequacy plane
Budget policy changes what is allowed.    -> spend plane
```

## Feasibility plane

`arkavo_router::planes::feasibility`

Consumes llama.cpp / runtime statistics only: `prompt_tokens` vs `n_ctx`,
KV-cache availability, queue depth, tokens/sec, thermal/load state, whether a
local model is loaded, and OOM / context overflow.

Allowed conclusions (`FeasibilityVerdict`):

- local can run now
- local can run slowly
- local cannot run
- local needs chunking
- local needs a smaller context

There is deliberately **no** "local answer is wrong, therefore spend cloud
dollars" conclusion. A slow laptop is not a budget-authorization event. A
feasibility failure may make the router tell the user it is busy, continue
locally, queue, retry locally, or return a partial answer — never silent spend.

Corrected escalation behavior:

- local is slow → tell the user local is busy / continue locally / ask before
  cloud (not: escalate to cloud)
- tokens/sec dropped → feasibility degraded, not quality degraded
- timeout → local unavailable; ask before spending, queue, retry locally, or
  return partial (not: cloud fallback)

## Quality / adequacy plane

`arkavo_router::planes::collapse`

This is the hard, unsolved product problem. v1 cannot judge whether an answer is
*good*; it can only catch visible **collapse**, so it is named a collapse
detector, not a quality gate. A clean verdict means "did not visibly fall
apart", not "this answer is adequate".

Visible collapse signals (`CollapseSignal`): empty output, truncated output,
repetition loop, format failure, tool-call schema failure, refusal on an allowed
task, obvious instruction noncompliance.

The honest v1 policy is: *local answer completed; cloud upgrade available; ask
the user before spending* — never *local answer seems bad, auto-spend cloud
budget*.

A detected collapse may **request** a cloud upgrade offer. It may not authorize
spend.

## Spend plane

`arkavo_budget::cloud_policy`

This plane alone decides whether cloud spend is allowed. Inputs: user policy,
remaining cap, per-request max, agent-loop budget, background-task budget, and
user confirmation state. It must not accept hardware/load signals as spend
justification — they are not parameters of `authorize_cloud_spend`.

```rust
enum CloudPolicy {
    LocalOnly,
    AskBeforeCloud,   // safe v1 default
    CloudWithinCap,
}
```

`CloudWithinCap` comes later, after the adequacy gate is validated against real
user outcomes.

## Router logic

1. Check local feasibility.
2. If local can run, run local.
3. If local visibly collapses, mark it as collapse.
4. If local completes, show the answer.
5. Offer a cloud upgrade when: the user asks, a collapse occurred, the task
   class is high-risk, and user policy allows asking.
6. Before any cloud call, check budget.
7. If budget is exhausted, stay local.

The key distinction: a collapse can trigger an upgrade *offer*; it does not
trigger automatic spend by default.

## The premise to measure

The cost-vs-time premise is an explicit measurement, not an assumption.
Instrument: local wait time, user abandonment, "upgrade to cloud" clicks, cloud
disabled, budget raised, budget lowered, retries after a slow local answer, task
category, and hardware class. The product question — at what latency do
cost-conscious, local-owning users prefer paid cloud — stays open until the data
answers it.

Until then the safe router posture is: bias local, do not auto-spend, make the
cloud upgrade explicit, and measure behavior.

## Final principle

> Availability changes what is possible.
> Quality changes what is advisable.
> Budget policy changes what is allowed.
