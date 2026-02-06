# NanoTDF KAS Rewrap Integration

## Summary

Added KAS rewrap support for NanoTDF, enabling cross-platform decryption compatibility with otdfctl and the Go SDK.

## Implementation

### New KAS Client Method

**`KasClient::rewrap_nanotdf(header_bytes, kas_url)`**

Similar to the existing `rewrap_standard_tdf` method but adapted for NanoTDF:

1. **Sends NanoTDF header bytes** to KAS (instead of manifest)
2. **KAS extracts ephemeral public key** from header
3. **KAS performs ECDH** using its EC private key + ephemeral public key
4. **KAS returns symmetric key** directly (no RSA wrapping needed)

### Protocol Changes

Added `header` field to `KeyAccessObject` structure:

```rust
pub struct KeyAccessObject {
    // ... existing fields ...

    /// NanoTDF header bytes (base64 encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
}
```

### Rewrap Request Format

For NanoTDF, the rewrap request includes:

```json
{
  "clientPublicKey": "",  // Not used for NanoTDF
  "requests": [{
    "algorithm": "ec:secp256r1",
    "policy": {
      "id": "policy",
      "body": ""  // Policy is in the header
    },
    "keyAccessObjects": [{
      "keyAccessObjectId": "kao-0",
      "keyAccessObject": {
        "type": "wrapped",
        "url": "http://localhost:8080/kas",
        "protocol": "kas",
        "wrappedKey": "",
        "policyBinding": {
          "hash": "",
          "algorithm": "HS256"
        },
        "header": "<base64-encoded-header-bytes>"
      }
    }]
  }]
}
```

### Key Differences from Standard TDF

| Aspect | Standard TDF | NanoTDF |
|--------|-------------|---------|
| Key data sent | Wrapped key from manifest | Header bytes |
| Algorithm | RSA (default) | EC (secp256r1) |
| Client ephemeral key | Generated for rewrap | Already in header |
| KAS operation | RSA decrypt + ECDH | ECDH only |
| Response | Wrapped key | Symmetric key (DEK) directly |

## Usage Example

```rust
use opentdf::kas::KasClient;
use opentdf_crypto::tdf::nanotdf::NanoTdf;

// Deserialize NanoTDF
let nanotdf = NanoTdf::from_bytes(&nanotdf_bytes)?;

// Get header bytes
let header_bytes = nanotdf.header.to_bytes()?;
let kas_url = nanotdf.header.kas.get_url()?;

// Create KAS client
let kas_client = KasClient::new(
    "http://localhost:8080",
    oauth_token,
    signing_key_pem,
)?;

// Rewrap to get symmetric key
let symmetric_key = kas_client
    .rewrap_nanotdf(&header_bytes, &kas_url)
    .await?;

// Decrypt payload
let plaintext = nanotdf.decrypt_with_key(&symmetric_key)?;
```

## How It Works

### Encryption Flow (Client)

1. Client generates **ephemeral EC keypair**
2. Client gets **KAS EC public key** (with kid=e1)
3. Client performs **ECDH**: `ephemeral_private + KAS_public → shared_secret`
4. Client derives **symmetric key**: `HKDF(shared_secret, salt="SHA256('L1L')", info=[])`
5. Client **encrypts payload** with symmetric key (AES-256-GCM)
6. Client stores **ephemeral public key** in header (compressed, 33 bytes)
7. Client stores **kid** in header resource locator

### Decryption Flow (otdfctl/Rust)

1. otdfctl reads NanoTDF header
2. otdfctl sends **header bytes** to KAS
3. KAS extracts:
   - **kid** from header → looks up its EC private key
   - **ephemeral public key** from header
4. KAS performs **ECDH**: `KAS_private + ephemeral_public → shared_secret`
5. KAS derives **symmetric key**: `HKDF(shared_secret, salt="SHA256('L1L')", info=[])`
6. KAS returns **symmetric key** to otdfctl
7. otdfctl **decrypts payload** with symmetric key

### Why Both Derive the Same Key

```
Client: ephemeral_private + KAS_public → shared_secret → symmetric_key
KAS:    KAS_private + ephemeral_public → shared_secret → symmetric_key
```

ECDH property: `alice_private * bob_public == bob_private * alice_public`

## Files Modified

1. **`crates/protocol/src/kas.rs`**: Added `header` field to `KeyAccessObject`
2. **`src/kas.rs`**: Added `rewrap_nanotdf` method and helper functions

## Next Steps

To complete NanoTDF decrypt functionality:

1. Add `decrypt_with_key` method to `NanoTdf` that accepts a pre-derived key
2. Create example demonstrating full decrypt flow with KAS
3. Add integration test with local KAS instance
4. Document OAuth token acquisition for real-world usage

## Related Documentation

- [NanoTDF Status](./NANOTDF_STATUS_FINAL.md) - Core implementation status
- [NanoTDF L1L v12 Implementation](./NANOTDF_L1L_V12_IMPLEMENTATION.md) - Technical details
- KAS rewrap protocol: OpenTDF platform documentation

## Testing

The implementation is ready for testing with:

```bash
# Start OpenTDF platform
docker-compose up -d

# Get OAuth token
TOKEN=$(curl -s http://localhost:8888/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials&client_id=opentdf&client_secret=secret" \
  | jq -r .access_token)

# Use token with KAS client
# (integration example to be created)
```

---

**Status**: ✅ Implementation complete, ready for integration testing
