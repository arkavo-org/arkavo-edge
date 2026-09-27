# arkavo-attestation

Platform attestation for arkavo-edge agents.

## Features

- **Platform Evidence Collection**: Securely gathers device identity, platform code, and security state.
- **Security State Detection**: Real-time detection of trusted, suspicious, or compromised system states.
- **Attestation Backends**:
  - macOS: an unsigned platform statement (IOPlatformUUID, serial, model, timestamp) collected via `ioreg` on hosts with a Secure Enclave. The Secure Enclave does not sign it, so it reports neither hardware binding nor freshness.
  - Linux, Windows and other hosts: a software fingerprint only; no TPM 2.0 backend exists.
- **Honest Reporting**: Truthful reporting of platform state to the control plane without local policy enforcement.
- **Cross-platform Support**: Unified attestation interface for macOS, Linux, Windows, and Raspberry Pi.