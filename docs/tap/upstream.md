# TAP upstream record and compatibility report

What the implementation in `crates/arkavo-tap*` was built against, what it was tested
against, and where it deliberately differs. Sources were retrieved on 2026-11-06 (UTC).

## Pinned sources

| Source | Revision | Retained hash (SHA-256 of the bytes retrieved) |
| --- | --- | --- |
| Visa TAP specification, `developer.visa.com/docs/trusted-agent-protocol/trusted-agent-protocol-specifications` | Page as served on 2026-11-06 (HTML, 709,786 bytes) | `f7023cbc1d9a8d121dec13d119b5cc0ff61e22374d3bbdc5101a82b08fcb54b3` |
| Visa public sample, `github.com/visa/trusted-agent-protocol` | `16d59bdf3f8a542bc538d0962edbb80ea30a02af` (HEAD) | Not used for fixtures |
| RFC 9421, HTTP Message Signatures | rfc-editor.org text | `612655786bf4293bfc486e4177571467fbb3de6e6f0eea90cb74c346a34fdf3c` |
| RFC 9530, Digest Fields | rfc-editor.org text | `544dbb7d9afceafa8c9931d9924ca6cff2b4807274166d7ed2483342ef2cdd6a` |
| RFC 8941, Structured Field Values (the version RFC 9421 uses) | rfc-editor.org text | `92e07becbe27974319d79de6195e72e25062b4193f68623e90ea4f7deaf33bb9` |
| RFC 9651, Structured Field Values (successor) | rfc-editor.org text | `fe27f2ec8819911afbe4bd11f6fcb947580da4c49e5423a1fff960e252ced26d` |
| AP2, `github.com/google-agentic-commerce/AP2` | `e1ea56db72a6385bce3e5c1112b3a56ce60acb43` (HEAD) | Not integrated |
| UCP (`crates/arkavo-ucp`) | This workspace, 0.107.0 | Not integrated in this phase |

The Visa page is the served HTML, so its hash changes whenever the page's markup does. The
normative rules below were read from that page and are restated here so a reviewer need not
trust the hash alone.

Errata: RFC 9421 has errata 8102 and 8103. Both replace `@signature-input` with
`@signature-params` in sections 7.2.8 and 7.5.3, which is what this implementation already
uses. No errata were listed for RFC 9530, RFC 8941 or RFC 9651.

## Profiles

| Profile | Covered components | `alg` | Parameters | Lifetime | Status |
| --- | --- | --- | --- | --- | --- |
| `arkavo-rfc9421-v1` | Read: `@authority` `@path` `@method` `@target-uri`. Checkout and commit: those plus `content-type` `content-digest` `idempotency-key` | `ed25519` (RFC 9421 registry) | `created` `expires` `nonce` `alg` `keyid` `tag`, all required, nothing else | at most 480 s; no future skew | Implemented |
| `visa-tap-pinned` | Read: `@authority` `@path` (the recognition baseline). Writes: the Arkavo write coverage, since the baseline binds neither method nor content | `Ed25519` (as the Visa page spells it) | The same six, `keyid` spelled in lower case | at most 480 s (Visa: 8 minutes) | Implemented for recognition signatures only |
| `visa-sample-fixture` | — | — | — | — | Not offered: no fixtures captured from the public sample |

Both profiles take Ed25519 keys only, published as OKP JWK `x` values (RFC 8037). A key is
identified by its RFC 7638 thumbprint for replay scope and grant binding.

`tag` carries the purpose: `agent-browser-auth` (browse) or `agent-payer-auth` (pay). Routes,
not requests, decide which purposes they accept, and neither purpose is a spending grant.

## Strictness beyond RFC 9421

- `Signature-Input` and `Signature` must each hold exactly one signature, under the same label.
- Both fields, and `Content-Digest`, must already be in canonical Structured Fields
  serialization. Structured Fields let a later duplicate key or parameter replace an earlier
  one; a field that does not re-serialize to itself is refused instead of repaired.
- Covered components are plain lower-case names with no parameters (`;sf`, `;key`, `;req` and
  so on are refused), each named once.
- `Content-Digest` accepts only `sha-256` and `sha-512`, and every listed digest must match.
- `@authority` and `@target-uri` come from the merchant's configured external origin, never from
  `Host`, `Forwarded` or `X-Forwarded-*`.
- Arkavo nonces carry at least 128 bits (22 or more base64url characters). The nonce is claimed
  only after the signature and digest verify.

## Known deviations and upstream ambiguities

- **`keyId` versus `keyid`.** The Visa page uses `keyid` in its parameter table, sample
  requests and signature-base code, and `keyId` in one browsing example. `keyId` is not a valid
  Structured Fields key (keys are lower case), so only `keyid` is accepted.
- **`Ed25519` versus `ed25519`.** The Visa examples spell the algorithm `Ed25519`; the RFC 9421
  registry name is `ed25519`. Each profile accepts only its own spelling.
- **Nonce form.** The Visa page fixes no nonce length or alphabet. The pinned profile accepts
  1 to 256 visible ASCII characters; the Arkavo profile requires 128 bits of base64url.
- **Consumer recognition and payment containers.** The Visa page defines their fields and their
  linkage by nonce and `kid`, but no canonical serialization for signing them, and its JSON
  examples do not parse. Both stay disabled: a principal is always reported unresolved, and no
  payment container is accepted (scenario T16).
- **Replay protection** is optional on the Visa page; it is always on here.
- **AP2** is not integrated. Configuration refuses `mode = "ap2"`, and order commitment
  requires a `local-test-grant` (scenario T27).

## The `httpsig` evaluation

`httpsig` 0.0.26 (the RFC 9421 crate the specification suggested evaluating) is pure Rust:
`ed25519-compact`, `p256`, `sfv` 0.15 and `sha2` 0.11, with no OpenSSL, `ring` or second TLS
stack. It is used here as an independent implementation in tests, in both directions, and not
as the verifier:

- It rebuilds `@signature-params` from parsed parameters in a fixed order (`created`,
  `expires`, `nonce`, `alg`, `keyid`, `tag`) instead of using the received serialization, so it
  cannot verify a signer that orders parameters differently, as the Visa examples do. This
  implementation uses the received text, and its own signer emits that fixed order so
  `httpsig`-based verifiers can check its signatures.
- It is a 0.0.x release.

## Compatibility report

These are separate results; none implies another.

| Result | Status | Evidence |
| --- | --- | --- |
| Arkavo profile passed | Passed | `arkavo-tap` unit and acceptance tests; `arkavo-tap-demo verify-suite` (25 scenarios end to end) |
| Published vectors passed | Passed | RFC 9421 appendix B.2.6: the signature base and the exact Ed25519 signature reproduced and verified with test-key-ed25519 (B.1.4). RFC 9530 example digests. RFC 8037 appendix A.3 thumbprint |
| Independent implementation passed | Passed, `httpsig` 0.0.26 only | `httpsig` verifies a signature made here, and this verifier accepts one made by `httpsig` (`tests/acceptance/interop.rs`) |
| Visa production onboarding completed | Not started | No Visa trust authority, key directory or credential is configured anywhere |

The development registry is a local trust authority. Registering a key there is not a Visa
registration.
