# arkavo-attestation

Platform attestation for arkavo-edge agents.

## Features

- **Platform Evidence Collection**: Securely gathers device identity, platform code, and security state.
- **Security State Detection**: Real-time detection of trusted, suspicious, or compromised system states.
- **Multiple Attestation Backends**:
  - TPM 2.0 hardware-backed quotes for Linux and Windows.
  - Secure Enclave hardware-backed attestation for macOS and iOS.
  - Software-based fingerprinting fallback for legacy or development environments.
- **Honest Reporting**: Truthful reporting of platform state to the control plane without local policy enforcement.
- **Cross-platform Support**: Unified attestation interface for macOS, Linux, Windows, and Raspberry Pi.