# opentdf-protocol

Protocol types and structures for OpenTDF (Trusted Data Format).

This crate provides the core data types for TDF manifests, policies, and KAS protocol
messages. It has no cryptographic operations or I/O - pure data structures only.

## Features

- TDF manifest structures (`TdfManifest`, `KeyAccess`, `PolicyBinding`)
- NanoTDF header and policy types
- KAS protocol request/response types
- Serde serialization support
- WASM compatible (with `js` feature for UUID)

## Usage

```rust
use opentdf_protocol::{TdfManifest, KeyAccess, PolicyBinding};

// Create a new TDF manifest
let manifest = TdfManifest::new(
    "0.payload".to_string(),
    "https://kas.example.com".to_string()
);

// Access manifest fields
println!("Payload: {}", manifest.payload.url);
```

## Part of the OpenTDF Workspace

This crate is part of the [opentdf-rs](https://github.com/arkavo-org/opentdf-rs) workspace:

- `opentdf` - High-level TDF API
- `opentdf-protocol` - Protocol types (this crate)
- `opentdf-crypto` - Cryptographic operations

## License

MIT OR Apache-2.0
