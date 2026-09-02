# Dispatch Gate

Every `tools/call` that passes through `arkavo mcp proxy` is admitted by three
local stages, in order. No stage does I/O; the p95 budget is 25ms
(`docs/gate-latency-baseline.md`). Each evaluation is recorded on the
process-local `dispatch_gate` tracker in the subsystem timing registry, which
means the samples are available to an embedder that hosts the proxy
in-process; a standalone `arkavo mcp proxy` has no sampler reading them.
Recording is in whole milliseconds, so a sub-millisecond evaluation — the
normal case — records as 0 ms.

## Trust model

Permits are issuer-signed CWTs (`docs/permit-cwt-schema.md`): the issuer's
key identifies the signer via the protected header's `kid`, and the
presenter's proof-of-possession key travels separately in the `cnf` claim
(RFC 8747). The COSE_Sign1 signature is verified against the issuer's key,
never against `cnf`.

The gate is configured with a list of trusted issuer public keys
(`GateConfig::trusted_issuers`). That list forms a single trust domain:
authn passes for a permit signed by *any* listed issuer. There is no
per-issuer policy and no binding to the permit's `iss` claim — two issuers
on the list are fully interchangeable as far as the gate is concerned. If
that granularity is ever needed, it is future work, not something
`trusted_issuers` provides today.

`arkavo_permit::decode` must never be used to make an authn decision: it
checks only claim structure, not the issuer or the signature.

## Wire format

The client places two base64url (no padding) strings under
`params._meta.arkavo`:

- `permit`: the CWT permit (`docs/permit-cwt-schema.md`)
- `pop`: the proof-of-possession signature over this invocation (same
  document, "Proof of Possession per Invocation")

A call without both is refused before any stage runs, and the refusal says
which of the four things went wrong — both absent, one present without the
other, a string that is not base64url, or a string longer than any permit can
be. All four are `authn:` refusals.

Both strings are bounded before they are decoded, at the encoded size of the
largest permit the parser accepts (16 KiB, so 21 849 characters).

An allowed `tools/call` has `_meta.arkavo` removed before the request is
forwarded upstream, so the permit and proof-of-possession never reach the
upstream server. Every other `_meta` key passes through unchanged; `_meta`
itself is dropped only if removing `arkavo` leaves it empty.

A `tools/call` sent as a notification (no `id`, so no response could ever
carry a denial back) cannot be policy-evaluated. The proxy drops it outright
— logging a warning — instead of forwarding it or answering it.

## Bounds on untrusted input

Everything a client can make the proxy allocate or hash is bounded, and each
bound answers rather than disconnects:

| Input | Bound | On breach |
|---|---|---|
| one JSON-RPC line | 1 MiB | `INVALID_REQUEST` (id null), the line is skipped, the connection continues |
| a JSON-RPC batch (top-level array) | not supported | `INVALID_REQUEST` (id null) with a warning, never silence |
| `_meta.arkavo.permit` / `.pop` | encoded size of a 16 KiB permit | `authn:` denial, without decoding |
| the permit itself | 16 KiB, nesting depth 16 | `authn:` denial from the parser |
| `arguments` | 256 KiB serialized | `policy:` denial, before either hash of them runs |

Server-initiated requests travel the other way and are not relayed to the
client in this slice: a `sampling/createMessage`, `elicitation/create` or
`roots/list` from the upstream is answered with JSON-RPC `-32601` so the
server learns at once, rather than blocking until the proxy's own per-request
timeout fires and takes the whole `tools/call` down with it.

## Stages

| Stage | Checks | Deny message prefix |
|---|---|---|
| authn | permit signature against the trusted-issuer list (by `kid`), `nbf`/`exp`/`iat` at now, proof-of-possession over the permit's identity, tool, and arguments against the `cnf` key | `authn:` |
| policy | permit's `policy_bundle_hash` equals the proxy's configured bundle; tool name and argument hash match the permit | `policy:` |
| budget | invocations of this permit (keyed on `Permit::id`, the digest of the permit's signed content, so re-encoding one permit cannot buy a second budget) stay below `budget.max_invocations` | `budget:` |

The proof-of-possession names the permit by that same `Permit::id`, so one
proof covers every valid encoding of one issuance — the same notion of "the
same permit" the budget counter uses.

### When budget is spent, and when it is returned

The counter increments when the gate admits the call, which is before the
call is dispatched. If the upstream never received it — the connection
failed, the request timed out — the proxy calls
`PolicyHook::on_forward_failed`, which returns the invocation via
`DispatchGate::refund`. A permit with a budget of one therefore survives a
transient upstream failure.

A call the upstream *ran* and answered with a JSON-RPC error is a completed
call: the tool did its work and reported a failure, and the invocation stays
spent. A refund never takes a counter below zero and never creates one, and
`refund_invocation` verifies the permit it credits rather than decoding it,
so a refund can only ever be aimed at a permit the caller can present.

A refused call returns JSON-RPC error `-32000` and never reaches the
upstream server.

The budget stage's usage table is bounded rather than unbounded per-permit
state: once it holds more than 4096 entries, expired counters are pruned
first, and if it is still over that threshold, entries are evicted by
soonest expiry until at most 3072 remain. A live (not-yet-expired) counter
can be evicted under that memory pressure — a caller can mint arbitrarily
many permits — but never otherwise.

## Running it

    arkavo mcp proxy --policy-bundle-hash <64 hex> --issuer-key <hex> [--issuer-key <hex> ...] [--hash sha256|blake3] -- <upstream command> [args...]

`--issuer-key` is repeatable and at least one is required; it becomes an
entry in `trusted_issuers`. Each value is hex-encoded and accepted in two
forms:

- 64 hex characters: a raw 32-byte Ed25519 public key.
- 130 hex characters: a raw 65-byte SEC1 uncompressed P-256 public key
  (`04 || x || y`).

## Not yet wired

Sequence-integrity (Epic 5.1), step-up approval (3.4), and closure receipts
(3.5) attach between the budget stage and `Allow`. Token and cost ceilings
in the permit budget are carried but not enforced at dispatch, because the
gate has no token counts.

Delegation chains are not walked: a permit's `parent_permit` hash is carried
through verification but the gate never resolves it, so it does not check
that the parent exists, is still valid, or has budget left. A delegated
permit is admitted on its own merits alone.

