# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| Latest release | Yes |
| Previous minor | Security fixes only |
| Older | No |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Use [GitHub Security Advisories](https://github.com/arkavo-org/arkavo-edge/security/advisories/new) to report vulnerabilities privately.

### What to include

- Description of the vulnerability
- Steps to reproduce
- Affected versions
- Impact assessment (if known)

### Response timeline

- **48 hours**: Acknowledgment of your report
- **7 days**: Initial assessment and severity classification
- **30 days**: Fix developed and tested
- **90 days**: Public disclosure (coordinated with reporter)

## Scope

The following are in scope for security reports:

- Authentication and authorization bypasses
- Injection vulnerabilities (command, SQL, XSS)
- Cryptographic weaknesses
- SSRF or network-level attacks
- Data leakage (PII, secrets, credentials)
- Denial of service via resource exhaustion

### Out of scope

- Vulnerabilities in third-party dependencies (report upstream; we monitor via `cargo-deny` and `cargo-audit`)
- Social engineering
- Physical access attacks

## Security Testing

The project includes automated security tests:

```bash
# Unit tests for security vulnerability fixes
cargo test -p arkavo-protocol --test security_vulnerabilities

# Mock provider PII detection tests
cargo test -p arkavo-cli mock_provider

# E2E DLP/PII leak detection
./tests/e2e_security_test.sh

# CLI security tests
./tests/security_cli_test.sh

# DLP/PII policy tests
./tests/dlp_pii_security_test.sh
```

## Security Maturity

This section is written for offensive-security reviewers and red-team readers: we list what is implemented and hardened, what is implemented but not yet cryptographically bound, and what is still on the roadmap.

### Implemented and hardened

The network and transport controls below (rustls-only TLS, SSRF egress filtering, per-IP rate limiting, host validation) are hardened and exercised by the security test suite. The data-protection and identity layers (TDF/KAS, ABAC, SwarmKit, DID:key) are implemented rather than independently audited.

- **OpenTDF / KAS encryption** — Tool outputs and SwarmKit role data can be wrapped in TDF envelopes; policy decisions are enforced by a Key Access Server before decryption. Built on `opentdf-rs` behind the optional `kas` build feature. Rewrap (decryption) needs a reachable KAS — the defaults are Arkavo-hosted (`identity.arkavo.net`, `100.arkavo.net`), but the endpoint is configurable, so a KAS self-hosted in-environment keeps encryption and policy enforcement fully functional offline / air-gapped.
- **ABAC / Attribute Release Policies** — Roles declare fine-grained attribute policies; the orchestrator builds role-scoped policies before any data reaches the role.
- **SwarmKit isolation** — Each role in a kit gets its own policy envelope; there are no shared blanket entitlements across a kit.
- **DID:key identity** — Each agent has a stable `did:key` identifier derived from an Ed25519 device keypair.
- **mDNS mesh** — Pure-Rust mDNS discovery and peer mesh with no dependency on system Avahi/Bonjour.
- **Local inference** — Gemma 4 and Ministral models run on-device via llama.cpp; routing and inference do not require cloud access.
- **DLP / PII scrubbing** — Pre-flight detection and redaction of sensitive patterns before data is sent to providers or logged.
- **TLS without OpenSSL** — All TLS uses `rustls` for musl compatibility and a reduced attack surface.
- **SSRF prevention** — Egress filtering blocks private and metadata IP ranges.
- **Rate limiting** — Per-IP rate limits on HTTP endpoints.
- **DNS rebinding protection** — Host validation on local servers.

### Implemented but not yet hardware-bound

- **Secure Enclave / TPM attestation** — The `arkavo-attestation` crate exists, compiles, and passes tests. On Apple Silicon it detects the Secure Enclave (`AppleKeyStore` / `AppleSEPKeyStore`), collects platform metadata via `ioreg`, and reports a security state (Trusted / Suspicious / Compromised / Unknown). However, the evidence is **not** a Secure-Enclave-signed quote; it is platform metadata plus a timestamp. The TPM backend (`AttestationType::TpmQuote`) is declared but not implemented; Linux and Windows currently fall back to software fingerprinting.
- **Key storage** — Device identity and agent Ed25519 keypairs are persisted on disk with filesystem permissions (`0o600`). They are **not** stored in the Secure Enclave, a non-extractable Keychain item, or a TPM yet.

### Roadmap

- Hardware-bound key generation and storage (Secure Enclave on macOS/iOS, TPM on Linux/Windows, StrongBox/KeyStore on Android where applicable).
- Cryptographically signed attestation quotes from the Secure Enclave / TPM that can be verified by a remote party.
- Feeding attestation evidence into the agent trust score, instead of treating an identity as verified once its `did:key` is known.

We ship the encryption, access-control, and identity layers now; the hardware-rooted trust layer is being built in the open and will be wired in once it can be verified end-to-end.
