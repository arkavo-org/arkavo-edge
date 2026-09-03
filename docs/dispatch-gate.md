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
other, a string that is not base64url, or a string longer than that field can
be. All four are `authn:` refusals.

Each string is bounded before it is decoded, by what that field can hold: the
permit by the encoded size of the largest permit the parser accepts (16 KiB,
so 21 849 characters), and the proof by the encoded size of one signature (64
bytes, so 89 characters, of which a real proof uses 86). Both bounds are
derived from `arkavo-permit`'s own constants — `MAX_PERMIT_BYTES` and
`SIGNATURE_BYTES` — rather than written out here or in the proxy.

An allowed `tools/call` has `_meta.arkavo` removed before the request is
forwarded upstream, so the permit and proof-of-possession never reach the
upstream server. Every other `_meta` key passes through unchanged; `_meta`
itself is dropped only if removing `arkavo` leaves it empty.

A `tools/call` sent as a notification (no `id`, so no response could ever
carry a denial back) cannot be policy-evaluated. The proxy drops it outright
— logging a warning — instead of forwarding it or answering it.

## Bounds on untrusted input

Neither side of the proxy's session — the client below it, the upstream
server above it — decides how much it allocates, hashes, or waits, and each
bound answers rather than disconnects:

| Input | Bound | On breach |
|---|---|---|
| one JSON-RPC line from the client | 1 MiB | `INVALID_REQUEST` (id null), the line is skipped, the connection continues |
| one JSON-RPC line from the upstream | 1 MiB | discarded with a warning, reading continues |
| a JSON-RPC batch (top-level array) | not supported | `INVALID_REQUEST` (id null) with a warning, never silence |
| refusals waiting to be written upstream | 16 queued | dropped and counted at `warn` — the first drop, then each doubling — so the reader never blocks |
| one write to the upstream's stdin | the per-request timeout | the write is abandoned, the connection is marked closed, `-32603`, and the invocation stays spent |
| `_meta.arkavo.permit` | encoded size of a 16 KiB permit (21 849 chars) | `authn:` denial, without decoding |
| `_meta.arkavo.pop` | encoded size of a 64-byte signature (89 chars, of which a real proof uses 86) | `authn:` denial, without decoding |
| the permit itself | 16 KiB, nesting depth 16 | `authn:` denial from the parser |
| `arguments` | 256 KiB serialized | `policy:` denial, before either hash of them runs, and only reachable once the permit has verified |

The issuer's key endpoint is on neither side of that session. Its two bounds
belong to `arkavo-cwt` and hold wherever a key set is fetched, not only under
this proxy:

| Input | Bound | On breach |
|---|---|---|
| the published key set | 64 KiB, nesting depth 16 | `CwtError::KeySet`, the body refused as it arrives |
| one fetch of the key set | 10 s, connection and body together | `CwtError::Fetch`; the refresh lock is held across the fetch, so every verification queued behind it waits no longer either |

The nesting bound is not the only one: ciborium, the decoder under `coset`,
refuses past 256 levels of its own accord. Sixteen is what this stack
tightens that to — a CWT is a shallow structure, 256 frames of recursion is
stack it has no use for, and the check is one iterative pass over the bytes
instead of the recursive descent it pre-empts.

Exactly two spans are walked besides the token's own outer structure: the
protected header (element 0 of the COSE_Sign1 array) and the payload (element
2), which are the only byte strings a decoder in this stack parses as CBOR in
their own right. Every other byte string — a signature, a `kid`, an EC
coordinate — reaches no decoder, so its contents are never walked and
ciborium's 256 is the floor there.

Those two slots must hold a *definite-length* byte string, and a token whose
header or payload is indefinite-length is refused outright. Its content is
spread over chunks, so there is no single span for the bound to hold on,
while ciborium concatenates the chunks and hands coset the result to decode
with a fresh 256-level budget — for the protected header, before any
signature is checked. RFC 8949 deterministic encoding forbids indefinite
lengths and nothing in this stack emits one, so the refusal costs no real
token.

Server-initiated requests travel the other way and are not relayed to the
client in this slice: a `sampling/createMessage`, `elicitation/create` or
`roots/list` from the upstream is answered with JSON-RPC `-32601` so the
server learns at once, rather than blocking until the proxy's own per-request
timeout fires and takes the whole `tools/call` down with it.

Which of the two an upstream message is comes from its shape and never from
its id. Anything carrying `method` — whatever type that field has, since a
non-string one is a badly named method and not an answer — is the server
asking, so a server that reuses the id of a call in flight, the way to have a
request of its own handed to the caller waiting on that id and relayed
downstream as the tool's result, gets the same `-32601` as any other. Only a
message with no `method` at all is matched against the requests in flight. The refusals themselves are queued
rather than written by the reader, so a server that asks faster than it reads
its own stdin cannot stop the proxy reading the response a caller is waiting
for.

It cannot stop the proxy writing, either. The refusal writer and every
forwarded request share one stdin behind one mutex, so a server that stops
reading its stdin blocks whoever holds that lock and everyone queued behind
it. Every write is bounded by the same per-request timeout: on expiry the
write is abandoned, the connection is marked closed — a partial line is
already in the pipe, and the next write would splice onto it — and the call
is answered with `-32603`. That failure keeps its invocation, because the
bytes the pipe did accept may have been a whole line the upstream ran.

## Stages

| Stage | Checks | Deny message prefix |
|---|---|---|
| authn | permit signature against the trusted-issuer list (by `kid`), `nbf`/`exp`/`iat` at now, proof-of-possession over the permit's identity, tool, and arguments against the `cnf` key | `authn:` |
| policy | permit's `policy_bundle_hash` equals the proxy's configured bundle; tool name and argument hash match the permit | `policy:` |
| budget | invocations of this permit (keyed on `Permit::id`, the digest of the permit's signed content, so re-encoding one permit cannot buy a second budget) stay below `budget.max_invocations` | `budget:` |
| capacity | the bounded usage table has room to count this permit at all | `capacity:` |

The last of those is not a fourth check so much as the budget stage having
nowhere to keep its count. It is named separately because the two say
opposite things to the holder: `budget:` means this permit is used up and
will not work again, `capacity:` means it has spent nothing and will work as
soon as the gate has room. Reported as one stage they were indistinguishable.

The proof-of-possession names the permit by that same `Permit::id`, so one
proof covers every valid encoding of one issuance — the same notion of "the
same permit" the budget counter uses.

### When budget is spent, and when it is returned

The counter increments when the gate admits the call, which is before the
call is dispatched. When no response comes back the proxy calls
`PolicyHook::on_forward_failed` with a `ForwardOutcome` saying how far the
call actually got, and only one of the two is refundable:

| `ForwardOutcome` | Upstream failures | Budget |
|---|---|---|
| `NotDelivered` | the connection was already closed, spawning failed, the write was cut short | returned via `DispatchGate::refund` |
| `MaybeExecuted` | the request timed out, the write timed out against a server that stopped reading, the flush failed, the upstream closed after the request was sent | stays spent, logged at `warn` |

The line is drawn at the write, not at the response. A timeout means the
request *was* delivered and a tool slower than the request timeout is still
running once the proxy stops waiting for it — refunding that would let any
such tool be invoked over and over on a budget of one, which is no budget at
all. Anything ambiguous is treated as "may have run". A permit with a budget
of one therefore survives an upstream that never took the call, and spends
its invocation on one that may have run.

A call the upstream *ran* and answered with a JSON-RPC error does not reach
this path at all: it is a completed call — the tool did its work and reported
a failure — and the invocation stays spent. A refund never takes a counter
below zero and never creates one, and `refund_invocation` verifies the permit
it credits rather than decoding it, so a refund can only ever be aimed at a
permit the caller can present.

A refused call returns JSON-RPC error `-32000` and never reaches the
upstream server.

The budget stage's usage table is bounded rather than unbounded per-permit
state: it counts at most 65 536 permits at once. Only *expired* counters are
ever dropped. Evicting a live one would restart a still-valid permit's count
at zero, which is precisely the second budget a flood of freshly minted
permits would be buying, so when pruning frees nothing the gate fails closed
instead: a permit the table is not already counting is denied at the capacity
stage with `gate capacity exhausted; retry after permits expire`, while every
permit already counted goes on being counted normally. Room returns as
entries expire — those permits are refused at authn from then on anyway. At
capacity, expiry pruning runs at most once per second; untracked permits are
refused until room frees — the scan is O(capacity), so a table already full
of live counters must not pay for it again on every call that finds nothing
to free.

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

