# NanoTDF otdfctl Compatibility - Detailed Debug Analysis

## Status

- ✅ Rust → Rust: **WORKS PERFECTLY**
- ❌ Rust → otdfctl: **FAILS** with `gcm.Open failed: cipher: message authentication failed`

## What We Know is Correct

### 1. HKDF Salt
**Spec Requirement**: `SHA256("L1L")` = `3de3ca1e50cf62d8b6aba603a96fca6761387a7ac86c3d3afe85ae2d1812edfc`

**Our Implementation**:
```rust
pub const NANOTDF_HKDF_SALT: [u8; 32] = [
    0x3d, 0xe3, 0xca, 0x1e, 0x50, 0xcf, 0x62, 0xd8,
    0xb6, 0xab, 0xa6, 0x03, 0xa9, 0x6f, 0xca, 0x67,
    0x61, 0x38, 0x7a, 0x7a, 0xc8, 0x6c, 0x3d, 0x3a,
    0xfe, 0x85, 0xae, 0x2d, 0x18, 0x12, 0xed, 0xfc,
];
```

Status: ✅ **VERIFIED CORRECT** - Matches spec constant exactly

### 2. HKDF Parameters
- Hash: SHA-256 ✅
- Info: empty `[]` ✅
- Length: 32 bytes ✅
- Salt: See above ✅

### 3. IV Handling
**Go Implementation**:
1. Creates 12-byte IV: `[9 zeros][3-byte random]`
2. Uses full 12 bytes for GCM encryption
3. Strips first 9 bytes from output: `cipherData[kIvPadding:]`
4. Payload contains: `[3-byte IV][ciphertext][tag]`

**Our Implementation**:
```rust
pub fn to_gcm_nonce(&self) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[9..12].copy_from_slice(&self.0);  // [9 zeros][3-byte IV]
    nonce
}
```

Payload serialization:
```rust
write_u24_be(writer, self.length)?;      // 3 bytes
writer.write_all(self.iv.as_bytes())?;   // 3 bytes
writer.write_all(&self.ciphertext_and_tag)?; // ciphertext + tag
```

Status: ✅ **CORRECT** - Matches Go implementation

### 4. Tag Size
**Header byte 26** (Symmetric & Payload Config): `0x01`
- Cipher enum: `0x01` = `Aes256Gcm96` = **12-byte tag**

**Our Implementation**:
```rust
let tag_size = TagSize::Bits96; // 12 bytes
```

Status: ✅ **CORRECT**

### 5. Binary Format
Comparing otdfctl-created vs Rust-created files:

| Field | Position | otdfctl | Rust | Status |
|-------|----------|---------|------|--------|
| Magic | 0-2 | `4c 31 4c` | `4c 31 4c` | ✅ Match |
| Protocol+ID | 3 | `0x10` | `0x10` | ✅ Match |
| Body length | 4 | `0x12` | `0x12` | ✅ Match |
| KAS URL | 5-22 | localhost:8080/kas | localhost:8080/kas | ✅ Match |
| KID | 23-24 | `65 31` (e1) | `65 31` (e1) | ✅ Match |
| ECC mode | 25 | `0x00` | `0x00` | ✅ Match |
| Sym config | 26 | `0x01` | `0x01` | ✅ Match |
| Policy type | 27 | `0x01` | `0x01` | ✅ Match |

Status: ✅ **ALL MATCH**

## What Might Be Wrong

### Theory #1: ECDH Shared Secret Mismatch

**Hypothesis**: Our ECDH calculation doesn't match Go's

**Evidence Against**:
- Rust → Rust works (same ECDH code)
- We use standard `p256` crate with `diffie_hellman()`
- Should be identical to Go's `ecdh.P256().ECDH()`

**Test Needed**:
- Log the shared secret bytes in both implementations
- Compare hex values

### Theory #2: KAS Public Key Parsing

**Hypothesis**: We're not parsing the KAS PEM correctly

**Our Code**:
```rust
let public_key = P256PublicKey::from_public_key_pem(kas_pem)?;
let public_bytes = public_key.to_encoded_point(true).as_bytes().to_vec();
```

**Potential Issue**: The PEM might contain extra data or we might be extracting the wrong bytes

**Test Needed**:
- Compare the actual public key bytes we use vs what Go uses
- Hex dump both

### Theory #3: KAS Doesn't Have Matching Private Key

**Hypothesis**: The KAS at kid="e1" rotated or uses different key

**Evidence**:
- We fetch from `/kas/v2/kas_public_key?algorithm=ec:secp256r1`
- Response says kid="e1"
- We include kid="e1" in header
- But KAS might not have that private key anymore

**Test Needed**:
- Check KAS logs to see if it's actually using the right key
- Try creating with otdfctl's key and comparing

### Theory #4: Cipher Configuration Mismatch

**Currently**: We always use `Aes256Gcm96` (cipher enum 0x01)

**But**: RustCrypto `aes-gcm` crate limitations:
- Only supports 12-16 byte tags (not 8-byte for enum 0x00)
- Always uses 12-byte nonces (internally)

**Potential Issue**: If otdfctl is using a different cipher mode or tag size than what we specify in the header

**Test Needed**:
- Create NanoTDF with enum 0x05 (128-bit tag) and test
- Check if otdfctl honors the enum byte

### Theory #5: AAD (Additional Authenticated Data)

**Currently**: Both sides use no AAD (verified)

**But**: What if there's implicit AAD from the header?

**Test Needed**:
- Check if Go passes any AAD to GCM

### Theory #6: Endianness in Shared Secret

**Hypothesis**: The shared secret bytes might be in different order

**Evidence**: ECDH shared secret is just raw bytes, should be identical

**Test Needed**:
- Compare shared secret bytes directly

## Recommended Debug Steps (In Order)

### Step 1: Verify HKDF Salt (DONE)
✅ Confirmed matches spec constant

### Step 2: Cross-Decrypt Test
```bash
# Test A: otdfctl → Rust
echo "test" > /tmp/test.txt
otdfctl encrypt /tmp/test.txt --tdf-type nano --policy-mode plaintext \
  --out /tmp/otdfctl-created.nanotdf \
  --host http://localhost:8080 --tls-no-verify \
  --with-client-creds '{"clientId":"opentdf","clientSecret":"secret"}'

# Try to decrypt with Rust (need to implement decrypt_from_file)
cargo run --example decrypt_nanotdf /tmp/otdfctl-created.nanotdf.tdf
```

### Step 3: Compare Ephemeral Keys
```bash
# Extract ephemeral public key from both files
# Position: after header (around byte 78-110)
xxd -s 78 -l 33 /tmp/test-with-kas-key.nanotdf
xxd -s 78 -l 33 /tmp/otdfctl-created.nanotdf.tdf
```

### Step 4: ECDH Parity Test
Create standalone test that:
1. Uses same KAS private key
2. Uses our ephemeral public key (from file)
3. Performs ECDH in both Rust and Go
4. Compares shared secret bytes
5. Compares HKDF output

### Step 5: Enable KAS Debug Logging
Check what key the KAS is actually using when otdfctl tries to decrypt our file

## Most Likely Root Cause

Based on the analysis, the most probable issue is:

**We're using a test ephemeral keypair that we generate, not coordinating with the KAS**

When we encrypt:
1. We generate random ephemeral keypair
2. We use KAS public key for ECDH
3. We store OUR ephemeral public in header

When otdfctl decrypts:
1. Reads OUR ephemeral public from header
2. Sends to KAS
3. KAS does ECDH(KAS_private, OUR_ephemeral_public)
4. KAS returns the derived key

**This should work!** The ECDH property guarantees both sides get the same shared secret.

**Unless**: The KAS public key we fetched doesn't match the private key the KAS is using.

## Next Action

1. ✅ Verify HKDF salt matches spec
2. Create decrypt example that works with otdfctl-created files
3. Compare the actual keys being used
4. Check KAS configuration for EC vs RSA default

