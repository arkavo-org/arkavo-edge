# NanoTDF Implementation - Next Session Guide

**Current Status:** WIP committed on branch `feature/nanotdf-implementation` (commit f161f34)

## Quick Start for Next Session

```bash
# Continue from where we left off
git checkout feature/nanotdf-implementation
cargo build --package opentdf-crypto  # Will show compilation errors to fix
```

## Compilation Errors to Fix (in order)

### 1. Fix `crates/crypto/src/tdf/nanotdf.rs` line 165
```rust
// WRONG:
.unwrap_or_else(|_| ResourceLocator::http("policy", 0));

// CORRECT:
.unwrap_or_else(|_| ResourceLocator::new(Protocol::Http, "policy".as_bytes().to_vec()));
```

### 2. Fix line 236 - EC KEM method name
```rust
// WRONG:
let (aes_key, ephemeral_public_key) = kem.encapsulate(kas_public_key)?;

// CORRECT:
let (aes_key, ephemeral_public_key) = kem.derive_key_with_ephemeral(kas_public_key)?;
```

### 3. Fix lines 282-289 - Header construction
```rust
// WRONG:
let header = Header {
    kas: kas_locator,
    ecc_mode: self.ecc_mode,
    config,
    policy,
    ephemeral_public_key,
    policy_binding: Zeroizing::new(vec![]),
};

// CORRECT:
use opentdf_protocol::nanotdf::header::EccAndBindingMode;

let ecc_and_binding_mode = EccAndBindingMode::new(false, self.ecc_mode);
let header = Header::new(
    kas_locator,
    ecc_and_binding_mode,
    config,
    policy,
    ephemeral_public_key.to_vec(),
)?;
```

### 4. Fix line 349 - Header field access
```rust
// WRONG:
let signature = if header.config.signature_mode.has_signature {

// CORRECT:
let signature = if header.symmetric_and_payload_config.signature_mode.has_signature {
```

### 5. Fix line 351 - Header field access
```rust
// WRONG:
.config.signature_mode.signature_ecc_mode

// CORRECT:
.symmetric_and_payload_config.signature_mode.signature_ecc_mode
```

### 6. Fix line 393 - EC KEM method name
```rust
// WRONG:
let aes_key = kem.decapsulate(kas_private_key, &self.header.ephemeral_public_key)?;

// CORRECT:
let aes_key = kem.derive_key_with_private(kas_private_key, &self.header.ephemeral_public_key)?;
```

### 7. Fix line 425 - Header field access
```rust
// WRONG:
let tag_size = match self.header.config.symmetric_cipher {

// CORRECT:
let tag_size = match self.header.symmetric_and_payload_config.symmetric_cipher {
```

## After Fixing Compilation Errors

### Test the implementation:
```bash
# Build with all features
cargo build --all-features

# Run unit tests
cargo test --package opentdf-crypto

# Run platform integration tests
cargo test --package opentdf -- --ignored platform
```

### Create a basic roundtrip test:
Add to `crates/crypto/src/tdf/nanotdf.rs` in the tests module:

```rust
#[cfg(all(test, feature = "kas"))]
mod integration_tests {
    use super::*;
    use p256::{SecretKey, pkcs8::EncodePublicKey};
    use rand::rngs::OsRng;

    #[test]
    fn test_nanotdf_roundtrip() {
        // Generate KAS key pair
        let kas_secret = SecretKey::random(&mut OsRng);
        let kas_public = kas_secret.public_key();
        let kas_public_bytes = kas_public.to_sec1_bytes();
        let kas_private_bytes = kas_secret.to_bytes();

        // Encrypt
        let plaintext = b"Hello, NanoTDF!";
        let nanotdf = NanoTdfBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(b"policy-content".to_vec())
            .encrypt(plaintext, &kas_public_bytes)
            .unwrap();

        // Serialize
        let bytes = nanotdf.to_bytes().unwrap();
        println!("NanoTDF size: {} bytes", bytes.len());

        // Deserialize
        let nanotdf2 = NanoTdf::from_bytes(&bytes).unwrap();

        // Decrypt
        let decrypted = nanotdf2.decrypt(&kas_private_bytes).unwrap();
        assert_eq!(plaintext, decrypted.as_slice());
    }
}
```

## Cross-Platform Testing with otdfctl

Check if otdfctl supports NanoTDF:
```bash
# Check otdfctl version and features
/Users/paul/Projects/opentdf/otdfctl/otdfctl --version

# Try to find NanoTDF examples or flags
/Users/paul/Projects/opentdf/otdfctl/otdfctl encrypt --help | grep -i nano
```

If otdfctl doesn't support NanoTDF, we'll need to test against:
- Go SDK examples in `/Users/paul/Projects/opentdf/platform/sdk`
- Reference implementations in the spec repo

## Current Implementation Stats

- **Lines of Code**: ~550 lines in nanotdf.rs
- **Crypto Backend**: RustCrypto (96-128 bit GCM tags)
- **Curves Supported**: P-256, P-384, P-521, secp256k1
- **Policy Binding**: GMAC (96-bit) ✅
- **Integration Tests**: 7 tests, 3 passing, 4 pending implementation

## Architecture Summary

```
opentdf-rs/
├── crates/
│   ├── protocol/          # Binary format definitions (no I/O)
│   │   ├── src/nanotdf/
│   │   │   ├── header.rs       # Header structures ✅
│   │   │   ├── policy.rs       # Policy types ✅
│   │   │   └── resource_locator.rs  # URL encoding ✅
│   │   └── src/binary/
│   │       └── mod.rs          # Big-endian I/O ✅
│   └── crypto/            # Cryptographic operations
│       ├── src/kem/ec.rs       # ECDH + HKDF ✅
│       └── src/tdf/
│           ├── nanotdf_crypto.rs      # AES-GCM (RustCrypto) ✅
│           ├── nanotdf_crypto_mbedtls.rs  # AES-GCM (Mbed TLS) 🚧
│           └── nanotdf.rs      # Main implementation 🚧
└── tests/
    └── platform_integration.rs  # Platform tests ✅
```

## Key Dependencies Added

- `opentdf-protocol` (workspace) - Binary format types
- EC curve libraries: p256, p384, p521, k256
- HKDF for key derivation
- Enabled `kas` feature by default

## Important Notes

1. **Default tag size is 96-bit** (12 bytes) - RustCrypto limitation
   - 64-bit tags require Mbed TLS (scaffolded, not complete)
   - Spec default is 64-bit, but we can't support it yet

2. **Policy binding** is stored in `Policy.binding` field, not separate `Header.policy_binding`

3. **Header structure** differs from initial design:
   - Uses `ecc_and_binding_mode: EccAndBindingMode` (1 byte bitfield)
   - Uses `symmetric_and_payload_config: SymmetricAndPayloadConfig` (1 byte bitfield)
   - No separate `ecc_mode` or `config` fields

4. **EC KEM API** uses:
   - `derive_key_with_ephemeral()` for encryption
   - `derive_key_with_private()` for decryption
   - NOT `encapsulate()`/`decapsulate()` (those return errors)

## Resources

- NanoTDF Spec: https://github.com/opentdf/spec/blob/main/schema/NanoTDF.md
- Issue tracking this work: #32
- Platform URL: http://localhost:8080
- Keycloak URL: http://localhost:8888
- Test credentials: `opentdf-sdk` / `secret`

## Contact/Questions

If you have questions when continuing this work:
1. Check the NanoTDF spec for protocol details
2. Look at EC KEM tests in `crates/crypto/src/kem/ec.rs` for usage examples
3. Check Header tests in `crates/protocol/src/nanotdf/header.rs` for bitfield encoding

Good luck! The foundation is solid, just needs the API corrections above. 🚀
