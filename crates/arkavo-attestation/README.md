# arkavo-attestation

Platform attestation for arkavo-edge agents.

## Purpose

Collects platform evidence and security state for device-bound agents. Supports multiple attestation backends with automatic fallback to software-based fingerprinting.

## Features

- **Platform Evidence Collection**: Device ID, platform code, security state, attestation type
- **Security State Detection**: Trusted, Suspicious, Compromised, Unknown
- **Multiple Attestation Types**:
  - TPM 2.0 Quote (Linux/Windows) - hardware-backed
  - Secure Enclave (macOS/iOS) - hardware-backed
  - Software Fingerprint (fallback) - software-based
- **Honest Reporting**: Reports platform state truthfully, no security theater
- **Cross-platform**: macOS, Linux, Windows, Raspberry Pi 5

## Architecture

arkavo-edge **reports** platform evidence honestly. Trust decisions are made by authnz-rs/arkavo-rs based on attestation type and security state.

```
┌─────────────────────────────────┐
│ PlatformAttestor trait          │
│  ├── collect_evidence()         │
│  ├── get_security_state()       │
│  └── get_capabilities()         │
└─────────────────────────────────┘
         ▲         ▲         ▲
         │         │         │
    ┌────┴──┐  ┌──┴───┐  ┌──┴──────┐
    │ TPM   │  │Secure│  │Fallback │
    │Quote  │  │Enclave│  │Software │
    └───────┘  └──────┘  └─────────┘
```

## Usage

```rust
use arkavo_attestation::{platform, detect_platform_code};
use arkavo_device_identity::AgentIdentity;

// Get device identity
let identity = AgentIdentity::new("0.38.2".to_string());
let platform_code = detect_platform_code(); // "macos-arm64", etc.

// Create attestor (auto-selects best available)
let attestor = platform::create_attestor(identity, platform_code);

// Collect evidence
let evidence = attestor.collect_evidence()?;

// Evidence contains:
// - device_id: Stable device identifier
// - platform_code: "macos-arm64", "linux-x86_64", etc.
// - platform_state: Trusted | Suspicious | Compromised | Unknown
// - attestation_type: TpmQuote | SecureEnclave | SoftwareFingerprint
// - evidence_blob: Platform-specific attestation data
// - app_version: Application version
// - timestamp: Unix timestamp
```

## Security State Detection

### macOS
- **Compromised**: Jailbreak detected (`/Applications/Cydia.app`, etc.)
- **Suspicious**: Debug mode active (`kern.bootargs` contains debug flags)
- **Trusted**: Clean system, no tampering detected

### Linux
- **Compromised**: Suspicious paths detected
- **Unknown**: Software attestation (honest reporting)

### Fallback
- **Unknown**: Always reports Unknown (no false confidence)

## Attestation Types

| Type | Hardware-Backed | Freshness | Use Case |
|------|----------------|-----------|----------|
| TPM Quote | ✅ | ✅ | Enterprise Linux/Windows |
| Secure Enclave | ✅ | ✅ | macOS/iOS (implemented) |
| Software Fingerprint | ❌ | ❌ | Development, unsupported platforms |

### macOS Secure Enclave

**Implementation Status**: ✅ Complete

The macOS backend uses `ioreg` to collect platform evidence:
- Checks for AppleKeyStore/AppleSEPManager presence
- Collects IOPlatformUUID, IOPlatformSerialNumber, model
- Includes timestamp for freshness
- Detects jailbreak and debug mode

**Limitations**: Currently uses `ioreg` output rather than true Secure Enclave cryptographic attestation. Future enhancement will use Security framework for signing with Secure Enclave keys.

## Design Principles

1. **Honest Reporting**: Never hide jailbreak, debug mode, or tampering
2. **No Local Policy**: Device reports state; authnz-rs decides trust level
3. **Graceful Degradation**: Falls back to software fingerprint if hardware unavailable
4. **Explicit Capabilities**: `AttestationCapabilities` clearly indicates what each backend supports

## Integration with NTDF

PlatformEvidence is serialized (currently JSON, migrating to binary per [specifications#1](https://github.com/arkavo-org/specifications/issues/1)) and sent to authnz-rs during authentication. The verifier wraps it in an NPE (Non-Person Entity) claim and issues an NTDF token.

## Testing

```bash
cargo test -p arkavo-attestation
```

All tests pass with zero clippy warnings.
