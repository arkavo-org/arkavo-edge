# OpenTDF Interoperability Testing

## Summary

opentdf-rs follows the **official OpenTDF specification** with camelCase field names in manifests.

## Compatibility Matrix

| Implementation | Manifest Format | Compatible with opentdf-rs | Notes |
|----------------|----------------|----------------------------|-------|
| **otdfctl (Go)** | camelCase | ✅ **YES** | Golden implementation, spec-compliant |
| **platform SDK (Go)** | camelCase | ✅ **YES** | Spec-compliant |
| **OpenTDFKit (Swift)** | snake_case | ❌ **NO** | Bug - uses non-standard snake_case |

## OpenTDF Spec Field Names

According to the official spec and otdfctl implementation:

```json
{
  "encryptionInformation": {    // ✅ camelCase (NOT encryption_information)
    "keyAccess": [...],           // ✅ camelCase (NOT key_access)
    "policyBinding": {...},       // ✅ camelCase (NOT policy_binding)
    "wrappedKey": "...",          // ✅ camelCase (NOT wrapped_key)
    "integrityInformation": {...} // ✅ camelCase (NOT integrity_information)
  },
  "payload": {
    "isEncrypted": true,          // ✅ camelCase (NOT is_encrypted)
    "mimeType": "..."             // ✅ camelCase (NOT mime_type)
  }
}
```

## Test Results

### otdfctl-created TDF (test_otdfctl_430.tdf)
```
✅ Successfully parsed manifest
✅ Encryption information read correctly
✅ Key access structures valid
✅ Policy binding present
✅ KAS URL: http://10.0.0.138:8080/kas
```

### OpenTDFKit-created TDF (test_swift.tdf)
```
❌ Parse error: missing field `encryptionInformation`
   Reason: Uses non-standard `encryption_information` (snake_case)
```

## Recommendation

**File bug against OpenTDFKit** to use spec-compliant camelCase field names.

Reference files:
- Compliant: `/Users/paul/Projects/arkavo/OpenTDFKit/test_otdfctl_430.tdf`
- Non-compliant: `/Users/paul/Projects/arkavo/OpenTDFKit/test_swift.tdf`

## Next Steps for KAS Testing

Since OpenTDFKit files are non-compliant, we should:

1. ✅ Create TDF files with opentdf-rs (spec-compliant)
2. Test KAS decryption with those files
3. Verify interoperability with otdfctl-created TDFs once we have:
   - Access to KAS at 10.0.0.138:8080
   - Valid OAuth token
   - TDF files created with proper KAS wrapping

## KAS Integration Status

- ✅ KAS client implementation complete
- ✅ Rewrap protocol (JWT signing, ECDH, HKDF, AES-GCM)
- ✅ Manifest parsing (spec-compliant)
- ⏳ Real KAS testing (requires OAuth token)
- ⏳ End-to-end decryption test (requires valid wrapped keys from real KAS)