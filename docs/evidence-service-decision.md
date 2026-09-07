# Evidence Service: Build vs Adopt Decision

**Status**: Spike result (Epic 0, item 3) — 2026-08-26
**Question**: For air-gapped customer deployments, adopt an existing SCITT transparency service or build a minimal Rust Merkle-log service on what we already have?

## In-repo assets (verified)

- `crates/arkavo-evofabric/src/merkle_tree.rs` (269 LOC): tested SHA-256 Merkle tree with inclusion proofs. **Not** CT/RFC 9162-shaped: no leaf/interior domain separation, duplicate-padding, and **no consistency proofs**. Reusable for batch roots; needs hardening before interop-grade receipts.
- `crates/arkavo-trust` (`src/jcs.rs`, `src/signing.rs`, `src/store.rs`): JCS canonicalization, Ed25519 signed events, append-only SQLite store. This is 90% of a transparency log already.
- No COSE/CWT/SCITT dependencies in `Cargo.lock`. AGENTS.md mandates minimal deps, rustls-only, pure-Rust preferred, and no C++ on Windows.

## Candidate survey

| Project | License | Runtime | Air-gapped fit |
|---|---|---|---|
| [microsoft/scitt-ccf-ledger](https://github.com/microsoft/scitt-ccf-ledger) | MIT | C++ on CCF, AMD SEV-SNP TEE (or virtual mode), Linux + Docker + Python | Poor. TEE hardware or a C++ confidential-computing stack; violates the Windows no-C++ rule; heavy ops for a single-tenant cluster. Best SCITT conformance available, but as a server to operate, not a library to embed. |
| [scitt-community/scitt-api-emulator](https://github.com/scitt-community/scitt-api-emulator) | MIT | Python | Poor. Reference/test harness for SCRAPI semantics, explicitly not production. Useful later as an interop fixture. |
| [google/trillian](https://github.com/google/trillian) | Apache-2.0 | Go, MySQL/Spanner backend, gRPC | Poor. Built for CT-scale multi-instance deployments; requires a database cluster; upstream is in maintenance mode ("no new features"). Massive overkill for an air-gapped cluster. |
| [transparency-dev/tessera](https://github.com/transparency-dev/tessera) (+ tesseract) | Apache-2.0 | Go, POSIX filesystem or MySQL | Moderate ops (POSIX mode is simple), but it targets the static-CT API, not SCITT receipts, and introduces a Go service into an all-Rust product. |
| [sigsum](https://www.sigsum.org/about/) | BSD-2-Clause (code), CC-BY-SA-4.0 (spec) | Go log server + cosigning witnesses | Simple, well-reasoned design; still a separate Go service to deploy, monitor, and witness-rotate in an offline network. |

Common thread: every candidate is a standalone service in Go/C++/Python assuming conventional ops tooling (Docker, managed DBs, internet-adjacent monitoring). None is a pure-Rust, embeddable, single-binary component that meets our dependency and platform constraints.

## Recommendation: build, with a phased commitment to the wire format

Build a minimal transparency service on `arkavo-trust`; do not adopt an external implementation.

**Phase 0 (zero new deps)** — ship value before committing to the COSE/SCITT wire format:

- Extend `arkavo-trust/src/store.rs` append-only events with a `prev_hash` field, forming a hash chain over the SQLite log (existing Ed25519 event signatures already cover tamper evidence per event).
- Batch events into epochs; compute an epoch Merkle root with `arkavo-evofabric`'s tree. First harden it: add RFC 6962-style domain separation (`0x00` leaf / `0x01` interior prefixes) and consistency proofs — a small, well-specified change to existing tested code.
- Sign each epoch root (tree size, root hash, timestamp) with the existing Ed25519 identity. That signed root **is** the phase-0 receipt; inclusion proofs come from the existing tree.
- Deliverable: internal audit evidence with append-only and inclusion guarantees, verifiable offline, no new dependencies, no new services.

**Phase 1 (only if customer interop demands it)** — emit COSE receipts per the IETF SCITT architecture (`draft-ietf-scitt-architecture` + COSE Receipts). This is when — and only when — we take a `cose`/`ciborium`-class dependency (pure Rust, rustls-compatible, no C++), and optionally validate against `scitt-api-emulator` as a test fixture. Phase-0 receipts remain verifiable; the COSE format becomes an export/interop layer over the same log.

## Key-custody model

The epoch-signing key is the root of trust for receipts; custody is the customer's choice:

- **Air-gapped (default)**: customer-managed HSM/KMS via **PKCS#11** (YubiHSM 2, Thales Luna, SoftHSM for dev/CI). The service holds only a PKCS#11 session handle; key material never leaves the HSM. Pure-Rust PKCS#11 clients exist (`cryptoki`), so no C++ is required — defer this dependency to the phase that needs HSM signing.
- **Connected deployments**: cloud KMS (AWS KMS / Azure Key Vault / GCP Cloud KMS) with Ed25519 or ECDSA signing APIs, behind the same `SigningKey` abstraction `arkavo-trust/src/signing.rs` already implies.
- **Phase 0 bootstrap**: existing software Ed25519 key in `arkavo-trust`, with a documented key-rotation path to HSM/KMS custody before receipts are treated as customer-facing evidence.

## Out of scope

- Witness cosigning / gossip (sigsum-style) for cross-customer split-view detection.
- SCRAPI REST surface; registration policies.
- Migration of `arkavo-evofabric` tree consumers to the hardened hash format (coordinate before changing domain separation).
