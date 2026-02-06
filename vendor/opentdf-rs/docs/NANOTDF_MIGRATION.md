# NanoTDF to TDF-CBOR Migration Guide

## Deprecation Notice

**NanoTDF is deprecated as of opentdf-rs 0.12.** New applications should use TDF-CBOR instead. NanoTDF will be removed in version 1.0.

### Why TDF-CBOR?

| Feature | NanoTDF | TDF-CBOR |
|---------|---------|----------|
| Format | Custom binary | Standard CBOR (RFC 8949) |
| Manifest | Binary-encoded, limited | Full TDF manifest, extensible |
| Policy | Embedded binding only | Full JSON policy with attributes |
| Assertions | Not supported | Fully supported |
| Multi-KAS | Not supported | Supported |
| Tooling | Custom parsers needed | Standard CBOR tools work |
| Size | ~250 bytes overhead | ~490 bytes overhead |
| Cross-SDK | Complex, version-sensitive | Simple, well-defined |

### When to Still Use NanoTDF

- Existing systems with NanoTDF infrastructure
- Extreme size constraints (< 500 bytes total overhead)
- Legacy compatibility requirements

---

## Migration Steps

### 1. Enable the CBOR Feature

Add the `cbor` feature to your `Cargo.toml`:

```toml
[dependencies]
opentdf = { version = "0.12", features = ["cbor"] }
```

### 2. Update Imports

```rust
// Old: NanoTDF (if using external nanotdf crate or custom implementation)
use nanotdf::{NanoTdf, Header, Payload};

// New: TDF-CBOR
use opentdf::{Policy, tdf_cbor::TdfCbor};
```

### 3. Encryption

#### NanoTDF (Deprecated)

```rust
// NanoTDF typically required manual construction
let header = Header::new(
    kas_url,
    kas_public_key,
    policy_binding,
    curve_type,
);

let payload = encrypt_payload(plaintext, symmetric_key)?;

let nanotdf = NanoTdf {
    header,
    payload,
    signature: None,
};

let bytes = nanotdf.to_bytes();
```

#### TDF-CBOR (Recommended)

```rust
use opentdf::{Policy, tdf_cbor::TdfCbor};

// Create policy with full attribute support
let policy = Policy::new(
    uuid::Uuid::new_v4().to_string(),
    vec![],  // data attributes
    vec!["user@example.com".to_string()],  // dissemination
);

// Build and encrypt in one fluent chain
let tdf_cbor = TdfCbor::encrypt(plaintext)
    .kas_url("https://kas.example.com")
    .kas_public_key(kas_public_key_pem)
    .policy(policy)
    .mime_type("application/octet-stream")
    .build()?;

// Serialize to CBOR bytes
let cbor_bytes = tdf_cbor.to_bytes()?;
```

### 4. Decryption

#### NanoTDF (Deprecated)

```rust
// Parse NanoTDF
let nanotdf = NanoTdf::from_bytes(&data)?;

// Extract header info for KAS rewrap
let kas_url = nanotdf.header.kas_url();
let wrapped_key = nanotdf.header.wrapped_key();

// Rewrap with KAS to get symmetric key
let symmetric_key = kas_client.rewrap(wrapped_key).await?;

// Decrypt payload
let plaintext = decrypt_payload(&nanotdf.payload, &symmetric_key)?;
```

#### TDF-CBOR (Recommended)

```rust
use opentdf::tdf_cbor::TdfCbor;

// Parse TDF-CBOR (automatically detects format via magic bytes)
let tdf_cbor = TdfCbor::from_bytes(&cbor_bytes)?;

// Access envelope information
println!("TDF type: {}", tdf_cbor.tdf);
println!("Version: {}.{}.{}",
    tdf_cbor.version[0],
    tdf_cbor.version[1],
    tdf_cbor.version[2]
);
println!("Algorithm: {}", tdf_cbor.manifest.encryption_information.method.algorithm);

// Get key access info for KAS rewrap
let key_access = &tdf_cbor.manifest.encryption_information.key_access[0];
let kas_url = &key_access.url;
let wrapped_key = &key_access.wrapped_key;

// After obtaining symmetric key from KAS...
let plaintext = tdf_cbor.decrypt_with_key(&symmetric_key)?;
```

### 5. Format Detection

TDF-CBOR includes magic bytes for reliable format detection:

```rust
use opentdf::tdf_cbor::{TdfCbor, is_tdf_cbor};

// Check if data is TDF-CBOR
if is_tdf_cbor(&data) {
    let tdf = TdfCbor::from_bytes(&data)?;
    // Process TDF-CBOR
} else if data.starts_with(b"PK") {
    // TDF Archive (ZIP)
} else if data.starts_with(b"{") {
    // Possibly TDF-JSON
}

// Or use the static method
if TdfCbor::has_magic_bytes(&data) {
    // It's TDF-CBOR
}
```

### 6. Policy Handling

#### NanoTDF (Deprecated)

```rust
// Policies were typically binary-encoded or remote URLs
let policy_binding = compute_policy_binding(&policy_bytes, &symmetric_key);
```

#### TDF-CBOR (Recommended)

```rust
use opentdf::Policy;

// Full policy with attributes and dissemination
let policy = Policy::new(
    uuid::Uuid::new_v4().to_string(),
    vec![
        // Attribute policies can be added here
    ],
    vec![
        "user@example.com".to_string(),
        "team@example.com".to_string(),
    ],
);

// With time constraints
use chrono::{Utc, Duration};

let policy = Policy::with_time_window(
    uuid::Uuid::new_v4().to_string(),
    vec![],
    vec!["user@example.com".to_string()],
    Some(Utc::now()),
    Some(Utc::now() + Duration::days(30)),
);

// Access policy from decrypted envelope
let policy_json = &tdf_cbor.manifest.encryption_information.policy;
```

### 7. Error Handling

#### NanoTDF Errors

```rust
match result {
    Err(NanoTdfError::InvalidHeader) => { /* ... */ }
    Err(NanoTdfError::DecryptionFailed) => { /* ... */ }
    // ...
}
```

#### TDF-CBOR Errors

```rust
use opentdf::tdf_cbor::TdfCborError;

match result {
    Err(TdfCborError::InvalidMagicBytes) => {
        // Data doesn't start with CBOR magic bytes
    }
    Err(TdfCborError::InvalidTdfIdentifier(id)) => {
        // Expected "cbor", got something else
    }
    Err(TdfCborError::CborDecodingFailed(reason)) => {
        // CBOR parsing failed
    }
    Err(TdfCborError::MissingField(field)) => {
        // Required field missing from envelope
    }
    // ...
}
```

---

## CLI Migration

If using the opentdf CLI:

### NanoTDF (Deprecated)

```bash
# NanoTDF commands (if available)
opentdf encrypt --format nano input.txt -o output.ntdf
opentdf decrypt --format nano output.ntdf -o recovered.txt
```

### TDF-CBOR (Recommended)

```bash
# TDF-CBOR commands
cargo run --example tdf_cbor_example --features cbor

# Or with custom tooling
opentdf encrypt --format cbor input.txt -o output.cbor
opentdf decrypt --format cbor output.cbor -o recovered.txt
```

---

## Size Comparison

For a 100-byte payload:

| Format | Total Size | Overhead |
|--------|------------|----------|
| NanoTDF | ~350 bytes | ~250 bytes |
| TDF-CBOR | ~590 bytes | ~490 bytes |
| TDF-JSON | ~1,275 bytes | ~1,175 bytes |
| TDF Archive (ZIP) | ~1,500 bytes | ~1,400 bytes |

TDF-CBOR is approximately **58% smaller** than TDF-JSON while providing full manifest capabilities.

---

## Cross-SDK Interoperability

TDF-CBOR provides excellent cross-SDK interoperability:

```rust
// Rust creates TDF-CBOR
let tdf_cbor = TdfCbor::encrypt(b"Hello from Rust")
    .kas_url("https://kas.example.com")
    .policy(policy)
    .build()?;

// Save for Swift SDK to read
std::fs::write("rust_created.cbor", tdf_cbor.to_bytes()?)?;

// Read Swift-created TDF-CBOR
let swift_bytes = std::fs::read("swift_created.cbor")?;
let swift_tdf = TdfCbor::from_bytes(&swift_bytes)?;
println!("Payload size: {} bytes", swift_tdf.payload.value.len());
```

Tested interoperability:
- Rust SDK (opentdf-rs) <-> Swift SDK (OpenTDFKit)
- Native CBOR manifest encoding in both SDKs
- Integer key mapping per TDF-CBOR specification

---

## Feature Comparison

### What You Gain with TDF-CBOR

1. **Standard Format**: CBOR is RFC 8949, widely supported
2. **Full Manifest**: Complete TDF manifest with all fields
3. **Assertions**: Support for signed assertions
4. **Multi-KAS**: Multiple key access objects supported
5. **Attributes**: Full attribute-based access control
6. **Better Tooling**: Standard CBOR tools can inspect files
7. **Cross-SDK**: Reliable interoperability with other SDKs

### What You Lose

1. **Size**: ~240 bytes more overhead than NanoTDF
2. **Simplicity**: Slightly more complex structure

---

## Migration Checklist

- [ ] Enable `cbor` feature in Cargo.toml
- [ ] Update imports to use `opentdf::tdf_cbor::TdfCbor`
- [ ] Replace NanoTDF encryption with `TdfCbor::encrypt().build()`
- [ ] Replace NanoTDF parsing with `TdfCbor::from_bytes()`
- [ ] Update error handling for `TdfCborError` types
- [ ] Test cross-SDK interoperability
- [ ] Update file extensions from `.ntdf` to `.cbor`
- [ ] Update documentation and comments

---

## Timeline

- **opentdf-rs 0.12**: NanoTDF marked as deprecated
- **opentdf-rs 0.x**: NanoTDF continues to work with deprecation warnings
- **opentdf-rs 1.0**: NanoTDF removed

---

## Need Help?

- [opentdf-rs GitHub Issues](https://github.com/opentdf/opentdf-rs/issues)
- [TDF-CBOR Specification](../../specifications/tdf-cbor/draft-00.md)
- [SDK Comparison](./SDK_COMPARISON.md)
