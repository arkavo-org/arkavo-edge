# NanoTDF Payload Length Bug - Fixed

## Summary

**Status**: ✅ **FIXED** - Cross-platform compatibility with otdfctl achieved!

## The Bug

The payload length field was being misinterpreted in both reading and writing operations.

### What Was Wrong

**Incorrect Understanding:**
- Payload length field = size of ciphertext + tag only
- IV was read/written separately after the length field

**Binary Format (Incorrect):**
```
[3-byte length][3-byte IV][ciphertext+tag]
```

### The Correct Format

**From Go SDK Analysis:**
Looking at `/Users/paul/Projects/opentdf/platform/sdk/nanotdf.go` lines 942-975:

```go
// Read payload length (includes IV + ciphertext + tag)
payloadLength := binary.BigEndian.Uint32(payloadLengthBuf)
cipherData := make([]byte, payloadLength)
_, err = n.reader.Read(cipherData)

// Extract IV from first 3 bytes of payload
iv := cipherData[:kNanoTDFIvSize]

// Decrypt using ciphertext from byte 3 onwards
decryptedData, err := aesGcm.DecryptWithIVAndTagSize(ivPadded, cipherData[kNanoTDFIvSize:], tagSize)
```

**Correct Understanding:**
- Payload length field = size of **IV + ciphertext + tag** (total payload bytes)
- IV is embedded as the first 3 bytes within the payload data

**Binary Format (Correct):**
```
[3-byte length: N][N bytes payload data: [3-byte IV][ciphertext][tag]]
```

## The Fix

### Reading (BinaryRead)

**Before:**
```rust
let length = read_u24_be(reader)?;
let mut iv_bytes = [0u8; 3];
reader.read_exact(&mut iv_bytes)?;  // Read IV separately
let mut ciphertext_and_tag = vec![0u8; length as usize];
reader.read_exact(&mut ciphertext_and_tag)?;  // Then read ciphertext+tag
```

**After:**
```rust
let length = read_u24_be(reader)?;  // Length includes IV + ciphertext + tag
let mut payload_data = vec![0u8; length as usize];
reader.read_exact(&mut payload_data)?;  // Read all payload data
let iv = NanoTdfIv::from_bytes([payload_data[0], payload_data[1], payload_data[2]]);
let ciphertext_and_tag = payload_data[3..].to_vec();
```

### Writing (Payload Creation)

**Before:**
```rust
let payload = NanoTdfPayload {
    length: ciphertext_and_tag.len() as u32,  // WRONG: missing IV length
    iv,
    ciphertext_and_tag,
};
```

**After:**
```rust
let payload = NanoTdfPayload {
    length: (3 + ciphertext_and_tag.len()) as u32,  // Includes 3-byte IV
    iv,
    ciphertext_and_tag,
};
```

## Files Modified

1. **`crates/crypto/src/tdf/nanotdf.rs`**:
   - Fixed `BinaryRead for NanoTdfPayload` (lines 515-544)
   - Fixed payload creation in encrypt method (line 325)

2. **`examples/decrypt_otdfctl_nanotdf.rs`**:
   - Removed debug hex dump (cleaner output)

## Verification

### Test 1: Parse otdfctl-created NanoTDF
```bash
$ cargo run --example decrypt_otdfctl_nanotdf
✓ Header parsed successfully
✓ All fields parsed correctly
```

### Test 2: otdfctl decrypt Rust-created NanoTDF
```bash
$ cargo run --example nanotdf_with_kas_key
Created: /tmp/test-with-kas-key.nanotdf

$ /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt \
    /tmp/test-with-kas-key.nanotdf \
    --host http://localhost:8080 --tls-no-verify \
    --with-client-creds '{"clientId":"opentdf","clientSecret":"secret"}'
Hello from Rust using real KAS key!
```

**Result**: ✅ **SUCCESS** - otdfctl successfully decrypted Rust-created NanoTDF!

## Impact

- ✅ **otdfctl → Rust**: Can now parse otdfctl-created NanoTDF files
- ✅ **Rust → otdfctl**: otdfctl can now decrypt Rust-created NanoTDF files
- ✅ **Rust → Rust**: Still works (internal consistency maintained)
- ✅ **Full cross-platform compatibility achieved!**

## Root Cause

The bug was introduced because the NanoTDF specification documentation was ambiguous about whether the payload length field includes the IV or not. The Go SDK implementation (the reference) includes it, but this wasn't immediately clear from reading the spec alone.

By examining the actual Go code and binary hex dumps of otdfctl-created files, we discovered:
- Byte offset calculation showed the payload length field value matched (IV + ciphertext + tag) total
- Go code explicitly reads all `payloadLength` bytes, then extracts IV from first 3 bytes

## Lessons Learned

1. **Spec ambiguity**: When documentation is unclear, always check the reference implementation
2. **Binary format debugging**: Hex dumps with manual byte offset calculation are invaluable
3. **Test with real data**: Creating test files with both implementations and comparing binary output catches format mismatches immediately

---

**Status**: Issue resolved. NanoTDF implementation is now fully compatible with otdfctl and the Go SDK.
