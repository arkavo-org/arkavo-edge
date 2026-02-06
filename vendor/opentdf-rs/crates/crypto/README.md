# opentdf-crypto

Cryptographic operations for OpenTDF with security hardening.

## Features

- **AES-256-GCM** encryption/decryption with segment support
- **RSA-OAEP** key encapsulation (via aws-lc-rs or RustCrypto)
- **EC key encapsulation** (P-256, P-384, P-521, secp256k1)
- **HMAC-SHA256** for policy binding and integrity verification
- **NanoTDF** encryption/decryption with ECDH key agreement
- **Automatic memory zeroization** for all sensitive data

## Security

- All key material uses `zeroize` to clear memory on drop
- Constant-time HMAC verification via `subtle::ConstantTimeEq`
- aws-lc-rs backend option for FIPS-validated RSA operations
- No timing side-channels in cryptographic comparisons

## Usage

```rust
use opentdf_crypto::TdfEncryption;

// Create encryption instance with generated keys
let tdf = TdfEncryption::new()?;

// Encrypt data
let encrypted = tdf.encrypt(b"sensitive data")?;

// Decrypt (using the same policy key)
let decrypted = TdfEncryption::decrypt_legacy(tdf.policy_key(), &encrypted)?;
```

## Feature Flags

- `aws-lc-provider` - Use aws-lc-rs for RSA (FIPS validated, recommended)
- `rustcrypto-provider` - Use RustCrypto RSA (has timing vulnerability)
- `kem-rsa` - RSA key encapsulation
- `kem-ec` - EC key encapsulation (P-256, P-384, P-521, secp256k1)
- `nanotdf` - NanoTDF support (requires `kem-ec`)

## Part of the OpenTDF Workspace

This crate is part of the [opentdf-rs](https://github.com/arkavo-org/opentdf-rs) workspace:

- `opentdf` - High-level TDF API
- `opentdf-protocol` - Protocol types
- `opentdf-crypto` - Cryptographic operations (this crate)

## License

MIT OR Apache-2.0
