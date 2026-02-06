# OpenTDF Interoperability Testing Summary

## Session Summary (Latest)

**Date**: 2025-10-03
**Focus**: Segment-based encryption and full interoperability achievement

**Key Achievements**:
1. ✅ Fixed policy JSON serialization (empty arrays → `null`)
2. ✅ Added `schemaVersion: "1.0"` to keyAccess objects
3. ✅ Fixed policy UUID format validation (must be proper 36-char UUID)
4. ✅ Updated example to use Policy struct serialization
5. ✅ Implemented segment-based encryption with GMAC integrity hashes
6. ✅ Implemented root signature generation (HMAC-SHA256)
7. ✅ Added `schemaVersion: "4.3.0"` to manifest root (critical for non-legacy validation)
8. ✅ **FULL INTEROPERABILITY ACHIEVED**: Rust TDFs decrypt successfully with otdfctl!
9. ✅ **Multi-segment validation**: Tested with 5MB file (3 segments), perfect decryption!

**Status**: ✅ COMPLETE - Rust opentdf-rs creates TDFs fully compatible with Go SDK/otdfctl.

**Test Results**:
- Small file (84 bytes, 1 segment): ✅ Decrypts successfully
- Large file (5MB, 3 segments): ✅ Decrypts successfully, MD5 verified
- All segment boundaries, GMAC hashes, and root signatures validated by otdfctl

## Test Environment
- **Platform**: OpenTDF Platform running on `localhost:8080`
- **Keycloak**: Running on `localhost:8888`
- **Rust Implementation**: opentdf-rs (current branch: `feature/kas-rewrap-protocol`)
- **Go Implementation**: otdfctl at `/Users/paul/Projects/opentdf/otdfctl`
- **Swift Implementation**: OpenTDFKit at `/Users/paul/Projects/arkavo/OpenTDFKit`

## Test Results

### ✓ KAS Client Infrastructure
- **Status**: PASSED
- **Test**: `cargo test --features kas test_kas_rewrap_with_real_server -- --ignored`
- **Result**: KAS client successfully connects to local platform
- **Details**:
  - KAS endpoint: `http://localhost:8080/kas`
  - Authentication: OAuth2 client credentials flow
  - Protocol: KAS v2 rewrap (Connect RPC compatible)

### ✓ KAS Public Key Retrieval
- **Status**: PASSED
- **Endpoint**: `http://localhost:8080/kas/v2/kas_public_key`
- **Result**:
  ```json
  {
    "publicKey": "-----BEGIN PUBLIC KEY-----\n...",
    "kid": "r1"
  }
  ```
- **Key Type**: RSA 2048-bit

### ✓ RSA Key Wrapping
- **Status**: IMPLEMENTED
- **Implementation**: RSA-OAEP with SHA1 hash (matching OpenTDF specification)
- **Location**: `src/crypto.rs::wrap_key_with_rsa_oaep()`
- **Example**: `examples/create_tdf_with_kas.rs` demonstrates usage

### ✅ Cross-Platform TDF Creation/Decryption
- **Status**: ✅ COMPLETE - Full interoperability achieved!
- **Scenarios tested**:
  1. **Rust → otdfctl (small file, 1 segment)**: ✅ **SUCCESS!**
     - File: 84 bytes plaintext
     - Result: Decrypted successfully, content matches
  2. **Rust → otdfctl (large file, 3 segments)**: ✅ **SUCCESS!**
     - File: 5MB plaintext (5,242,880 bytes)
     - Segments: 3 (2MB, 2MB, 1MB)
     - Result: Decrypted successfully, MD5 checksum verified
  3. **otdfctl → Rust**: ⚠️ Blocked (requires segment-based decryption implementation)
  4. **OpenTDFKit → Rust**: ⏸ (Not yet tested)
  5. **Rust → OpenTDFKit**: ⏸ (Not yet tested)

**Issues Discovered and Fixed:**
- ✅ **Policy Binding Encoding** (FIXED):
  - **Root Cause**: Go SDK uses `base64(hex(hmac))` but Rust was using `base64(hmac)`
  - **Go SDK** (`/platform/sdk/tdf.go:537-538`): Hex encodes HMAC before base64
  - **Rust** (was): Directly base64 encoded the 32-byte HMAC
  - **Fix**: Updated `src/manifest.rs:generate_policy_binding_raw()` to match Go format
  - **Result**: Policy bindings now use same encoding (64 hex chars → base64)

- ✅ **Policy JSON Serialization** (FIXED):
  - **Root Cause**: Empty arrays serialized as `[]` in Rust but `null` in Go SDK
  - **Impact**: Different JSON serialization affects HMAC calculation for policy binding
  - **Fix**: Added custom serde serialization in `src/policy.rs:serialize_empty_vec_as_null()`
  - **Result**: Empty `dataAttributes` and `dissem` now serialize as `null`

- ✅ **Schema Version** (FIXED):
  - **Root Cause**: Missing `schemaVersion` field in `keyAccess` object
  - **Go SDK**: Includes `"schemaVersion": "1.0"` in keyAccess
  - **Fix**: Added `schema_version` field to `KeyAccess` struct with default value "1.0"
  - **Result**: Manifest now includes schemaVersion matching Go SDK

- ✅ **Policy UUID Format** (FIXED):
  - **Root Cause**: Used short policy ID `"policy-1"` instead of proper UUID
  - **KAS Validation**: Expects 36-character UUID format (e.g., `"452a039c-a0c2-11f0-92a3-e6f7c0fa8b99"`)
  - **Error**: "invalid_argument: request error" from KAS
  - **Fix**: Updated example to use proper UUID format `"00000000-0000-0000-0000-000000000000"`
  - **Result**: KAS now accepts the policy, progresses past unwrap stage

- ✅ **Manifest Schema Version** (FIXED):
  - **Root Cause**: Missing top-level `schemaVersion` field caused Go SDK to treat TDF as legacy
  - **Legacy Format**: Expects hex-encoded HMAC in root signature
  - **Modern Format**: Uses raw HMAC bytes (base64 encoded)
  - **Fix**: Added `schemaVersion: "4.3.0"` to TdfManifest
  - **Result**: Go SDK now correctly validates as modern (non-legacy) TDF

- ✅ **Segment-based Encryption** (IMPLEMENTED):
  - **Implementation**: `TdfEncryption::encrypt_with_segments()` in `src/crypto.rs`
  - **GMAC Extraction**: Last 16 bytes of AES-GCM ciphertext (the authentication tag)
  - **Segment Storage**: IV (12 bytes) + ciphertext + tag (16 bytes)
  - **Root Signature**: HMAC-SHA256(payloadKey, concat(gmac_tags))
  - **Result**: Generates manifests identical to Go SDK format

## Components Verified

### KAS Rewrap Request Format ✓
```rust
UnsignedRewrapRequest {
    client_public_key: String,  // PEM format
    requests: Vec<PolicyRequest>
}
```

### KAS Rewrap Response Format ✓
```rust
RewrapResponse {
    responses: Vec<PolicyRewrapResult>,
    session_public_key: Option<String>
}
```

### ECDH Key Unwrapping ✓
- Protocol: ECDH → HKDF(salt=SHA256("TDF")) → AES-256-GCM
- Implementation: Verified in `src/kas.rs:456`

## Implementation Progress

### ✅ Phase 1: RSA Key Wrapping (COMPLETED)
- [x] Add `rsa` crate dependency (Cargo.toml)
- [x] Add `sha1` crate for OAEP padding
- [x] Create `src/kas_key.rs` module for KAS public key fetching
- [x] Implement `wrap_key_with_rsa_oaep()` function in `src/crypto.rs`
- [x] Create example `examples/create_tdf_with_kas.rs`
- [x] Successfully fetch KAS public key from platform
- [x] Successfully wrap payload key with RSA-OAEP-SHA1

### ✅ Phase 2: Policy Binding Compatibility (COMPLETED)
- [x] Add `hex` crate dependency (Cargo.toml)
- [x] Investigate Go SDK policy binding generation
- [x] Update `generate_policy_binding_raw()` to use hex encoding
- [x] Verified new format: base64(hex(hmac)) - 64 hex chars after b64 decode
- [x] Confirmed compatibility with Go SDK format

### ✅ Phase 3: Manifest Compatibility (COMPLETED)
- [x] Fix policy JSON serialization (empty arrays → null)
- [x] Add `schemaVersion` field to keyAccess (value: "1.0")
- [x] Fix policy UUID format (use proper 36-char UUID)
- [x] Update example to use Policy struct serialization
- [x] Verified KAS accepts rewrap requests (no more "bad request" errors)

### ✅ Phase 4: Segment-based Encryption (COMPLETED)
- [x] Implement `encrypt_with_segments()` in `src/crypto.rs`
- [x] Add GMAC tag extraction (last 16 bytes of AES-GCM output)
- [x] Implement `generate_root_signature()` in `src/manifest.rs`
- [x] Add `add_entry_with_segments()` to archive builder
- [x] Update example to use segment-based encryption
- [x] Add top-level `schemaVersion: "4.3.0"` to manifest
- [x] **Verified otdfctl successfully decrypts Rust TDFs!**

## Next Steps for Additional Testing

### 1. Complete Cross-Platform Testing
- [x] ✅ Verify Rust TDF decrypts successfully with otdfctl
- [ ] Test otdfctl-created TDF decrypts with Rust (segment-based decryption)
- [ ] Test OpenTDFKit interoperability (both directions)
- [ ] Validate policy binding across all implementations
- [ ] Test attribute-based access control interoperability

### 2. Implement Segment-based Decryption
- [ ] Update `TdfEncryption` to support reading segmented payloads
- [ ] Validate GMAC hashes during decryption
- [ ] Verify root signature before decrypting
- [ ] Handle both legacy (single-payload) and modern (segmented) formats

### 3. Additional Features
- [ ] Support for larger files with multiple segments
- [ ] Streaming decryption for large TDFs
- [ ] Attribute-based policies in TDF creation
- [ ] Assertion support

## Code References
- KAS client implementation: `src/kas.rs`
- KAS integration tests: `tests/kas_integration.rs`
- Example usage: `examples/kas_decrypt.rs`
- Recent commit: `a2af789` - Fix KAS rewrap request format for Connect RPC compatibility

## OAuth Token Generation
```bash
curl -s http://localhost:8888/auth/realms/opentdf/protocol/openid-connect/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials&client_id=opentdf&client_secret=secret" \
  | jq -r '.access_token'
```

## Platform Configuration
```bash
curl -s http://localhost:8080/.well-known/opentdf-configuration | jq
```
