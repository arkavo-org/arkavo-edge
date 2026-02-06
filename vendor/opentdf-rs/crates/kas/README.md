# opentdf-kas

Server-side cryptographic operations for OpenTDF Key Access Service (KAS) implementations.

## Overview

This crate provides the cryptographic primitives needed to implement a KAS server that can handle both NanoTDF (EC-based) and Standard TDF (RSA-based) key unwrap and rewrap operations.

## Features

- **`ec`** (default): EC P-256 support for NanoTDF
  - ECDH key agreement with x-coordinate extraction
  - NanoTDF version detection and salt computation
  - HKDF + AES-GCM key rewrapping

- **`rsa`**: RSA-2048 support for Standard TDF
  - RSA-OAEP decryption (SHA-1 for OpenTDF compatibility)
  - AES-GCM key rewrapping

## Usage

```rust
use opentdf_kas::{KasEcKeypair, ec_unwrap};

// Generate or load KAS EC keypair
let kas_keypair = KasEcKeypair::generate()?;

// Get public key for clients
let public_key_pem = kas_keypair.public_key_pem();

// Perform NanoTDF rewrap (called during KAS rewrap request)
let rewrapped_key = ec_unwrap(
    &nanotdf_header,
    &ephemeral_public_key,
    kas_keypair.private_key(),
    &session_shared_secret,
)?;
```

## NanoTDF Version Support

Supports NanoTDF versions 1.2 ("L1L") and 1.3 ("L1M") with automatic version detection from header bytes. Salt computation follows the NanoTDF spec: `salt = SHA256(MAGIC + VERSION)`.

## Security Notes

### RSA-OAEP with SHA-1

The RSA unwrap implementation uses SHA-1 for OAEP padding. This is intentional for compatibility with the OpenTDF specification and existing TDF files. SHA-1 in OAEP context does not have the same collision vulnerabilities as in signature contexts.

### EC Public Key Format

EC public keys are encoded in SEC1 format within PEM blocks. For maximum interoperability with other systems, consider using SPKI (SubjectPublicKeyInfo) format when exchanging keys externally.

## License

MIT OR Apache-2.0
