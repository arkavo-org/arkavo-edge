# CWT Permit Schema for Permit-Bound Tool Execution

A permit is a CBOR Web Token (RFC 8392) signed as COSE_Sign1 (RFC 8152) that
binds a single tool invocation — tool name plus canonicalized arguments — to
an agent workload, a policy bundle, an execution budget, and a
proof-of-possession key. The `arkavo-permit` crate (`crates/arkavo-permit`)
implements this schema.

## Wire Format

```
CWT = #6.61( COSE_Sign1 )        ; CBOR tag 61 wrapping tag 18
COSE_Sign1 payload = claims set  ; CBOR map, labels below
```

The COSE_Sign1 protected header carries:

- `alg` (1): `EdDSA` (-8) or `ES256` (-7). No other algorithms are accepted.
- `content type` (3): `application/cwt` (CoAP content format 61).

## Claims

Standard claims use their RFC 8392 / RFC 8747 keys. Arkavo claims use
private-use keys below -65536 (RFC 8392 section 4).

| Key | Name | Type | Req | Semantics |
|-----|------|------|-----|-----------|
| 1 | iss | tstr | required | Permit issuer identity (URI or DID). |
| 2 | sub | tstr | required | Delegator: the human identity on whose behalf the agent acts. |
| 4 | exp | uint | required | Expiration, seconds since UNIX epoch. Permits are rejected at `now >= exp`. |
| 5 | nbf | uint | required | Not-before, seconds since UNIX epoch. Must satisfy `nbf < exp`. |
| 6 | iat | uint | required | Issued-at, seconds since UNIX epoch. Permits with `iat > now` are rejected. |
| 8 | cnf | map | required | RFC 8747 confirmation claim: `{1: COSE_Key}` — the proof-of-possession key the signature is verified against. |
| -70001 | agent_workload_id | tstr | required | Agent workload identity (SPIFFE-style workload ID). |
| -70002 | policy_bundle_hash | bstr | required | Hash of the policy bundle authorizing this permit. |
| -70003 | tool_name | tstr | required | Fully-qualified tool name this permit authorizes. |
| -70004 | argument_hash | bstr | required | Hash of the canonicalized tool arguments (see canonicalization below). |
| -70005 | data_classifications | [+ tstr] | required | TDF data-classification attributes the invocation may touch. May be an empty array. |
| -70006 | budget | map | required | Execution budget; sub-keys below. |
| -70007 | sequence_state_hash | bstr | required | Hash of the sequence state (replay/ordering anchor). |
| -70008 | parent_permit | bstr | optional | Hash of the parent permit's CWT bytes for A2A delegation chains. |

Budget map (claim -70006) sub-keys:

| Key | Name | Type | Req | Semantics |
|-----|------|------|-----|-----------|
| 1 | max_invocations | uint | required | Maximum number of tool invocations; must be >= 1. |
| 2 | token_ceiling | uint | optional | Ceiling on total tokens consumed across invocations. |
| 3 | cost_micro_usd | uint | optional | Cost ceiling in micro-USD (10^-6 USD). |

Unknown claims are ignored for forward compatibility; every required claim
must be present and correctly typed or the permit is rejected (fail closed).

## Confirmation Key (`cnf`)

The `cnf` claim follows RFC 8747: a map with key `1` holding a COSE_Key.
Supported key types:

- `OKP` (1) with curve `Ed25519` (6) and the `x` parameter (-2): the 32-byte
  public key. Used with `EdDSA`.
- `EC2` (2) with curve `P-256` (1) and `x` (-2) / `y` (-3) parameters: 32-byte
  affine coordinates. Used with `ES256`.

The COSE_Key's key type and curve must agree with the `alg` in the protected
header, and the COSE_Sign1 signature must verify against this key. A permit
therefore proves possession of the confirmation key at mint time; verifiers
never need out-of-band key lookup to check the signature, and policy layers
can pin the `cnf` key to a registered workload identity.

## Canonicalization of Tool Arguments

`argument_hash` (-70004) commits the permit to one exact tool invocation.
Arguments are canonicalized before hashing so that semantically identical
JSON produced by different serializers hashes identically:

- UTF-8 output with no insignificant whitespace.
- Object keys sorted by Unicode code point, recursively.
- Strings escaped per RFC 8259 section 7: short escapes (`\"`, `\\`, `\b`,
  `\f`, `\n`, `\r`, `\t`) where defined, `\u00XX` for other control
  characters; all other characters emitted as raw UTF-8.
- Numbers rendered with `serde_json::Number`'s shortest representation
  (integers without fraction, floats shortest round-trip).

Example: `{"max_bytes":4096,"path":"/tmp/data.csv"}` is the canonical form of
any key ordering and whitespace layout of that object.

## Hash Algorithms

Permit-bound hashes (policy bundle, arguments, sequence state, parent permit)
are computed with a pluggable algorithm:

- `sha256` (default)
- `blake3` (permitted alternative)

The algorithm is not recorded in the token: hashes are opaque byte strings,
and the relying party knows which algorithm its policy requires. Mixing
algorithms across the claims of one permit is discouraged.

## Signing Algorithms

- `EdDSA` over Ed25519 is the primary choice: compact keys (32 bytes),
  deterministic signatures, and no ECDSA nonce concerns.
- `ES256` over P-256 is supported for environments that require NIST curves
  (e.g. iOS Secure Enclave). ES256 signature values use the IEEE P1363
  fixed-size `r || s` encoding (64 bytes) required by RFC 8152 section 8.1.

## Expiry Guidance

Permits are single-purpose and short-lived: `exp - nbf` should be on the
order of seconds to a few minutes (the test vectors use 300 s), just long
enough to cover minting, transport, and execution. Long-lived authority
belongs in the referenced policy bundle (-70002), not in the permit itself.
Verifiers must check `nbf <= now < exp` and `iat <= now`, and must reject
`nbf >= exp` structurally.

## A2A Delegation Chains (`parent_permit`)

When an agent delegates work to another agent (A2A), the delegated permit
carries `parent_permit` (-70008): the hash of the parent permit's complete
CWT bytes (tag 61 included). This binds the child to the exact parent token,
so a verifier can:

- fetch the parent CWT, hash it, and compare against the claim;
- recursively verify the parent (signature, window, budget);
- enforce attenuation: the child's tool, classifications, and budget must be
  a subset of the parent's authority.

Chain length limits and attenuation rules are policy decisions enforced
above this crate; the schema only provides the cryptographic link.

## Test Vectors

`crates/arkavo-permit/tests/vectors/` holds reproducible vectors (Ed25519 +
SHA-256, ES256 + BLAKE3, and a parent-chained A2A permit), each containing
the signed CWT (hex), the expected claims, the public key, and the secret
key. Regenerate with:

```bash
cargo run -p arkavo-permit --example generate_vectors
```

Generation is deterministic (fixed keys and timestamps), so regeneration
must leave the committed files unchanged. `tests/vectors_test.rs` verifies
each vector end to end.
