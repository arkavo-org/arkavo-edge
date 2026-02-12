# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| Latest release | Yes |
| Previous minor | Security fixes only |
| Older | No |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Use [GitHub Security Advisories](https://github.com/arkavo/arkavo-edge/security/advisories/new) to report vulnerabilities privately.

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

## Security Design Principles

- **No OpenSSL**: All TLS uses `rustls` for musl compatibility and reduced attack surface
- **Egress filtering**: SSRF prevention blocks private/metadata IP ranges
- **Rate limiting**: All HTTP endpoints enforce per-IP rate limits
- **Host validation**: DNS rebinding protection on local servers
- **DLP scrubbing**: PII detection and redaction in LLM responses
