# Dispatch Gate

Every `tools/call` that passes through `arkavo mcp proxy` is admitted by three
local stages, in order. No stage does I/O; the p95 budget is 25ms
(`docs/gate-latency-baseline.md`), and the observed latency is recorded on
`dispatch_gate` in the subsystem timing registry and shown in the AG-UI
health panel.

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

A call without both is refused before any stage runs.

An allowed `tools/call` has `_meta.arkavo` removed before the request is
forwarded upstream, so the permit and proof-of-possession never reach the
upstream server. Every other `_meta` key passes through unchanged; `_meta`
itself is dropped only if removing `arkavo` leaves it empty.

A `tools/call` sent as a notification (no `id`, so no response could ever
carry a denial back) cannot be policy-evaluated. The proxy drops it outright
— logging a warning — instead of forwarding it or answering it.

## Stages

| Stage | Checks | Deny message prefix |
|---|---|---|
| authn | permit signature against the trusted-issuer list (by `kid`), `nbf`/`exp`/`iat` at now, proof-of-possession over the permit, tool, and arguments against the `cnf` key | `authn:` |
| policy | permit's `policy_bundle_hash` equals the proxy's configured bundle; tool name and argument hash match the permit | `policy:` |
| budget | invocations of this permit (keyed on `Permit::id`, the digest of the permit's signed content, so re-encoding one permit cannot buy a second budget) stay below `budget.max_invocations` | `budget:` |

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
