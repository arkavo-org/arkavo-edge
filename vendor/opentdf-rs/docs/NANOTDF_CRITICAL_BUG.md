# ~~CRITICAL BUG: Resource Locator Identifier Position~~

## ✅ **RESOLVED** - Actual Bug Was Payload Length Field

**Original Hypothesis**: Resource Locator identifier bytes are in wrong position
**Actual Bug**: Payload length field interpretation was incorrect
**Status**: **FIXED** - See `NANOTDF_PAYLOAD_LENGTH_BUG_FIX.md` for details

---

## Original Analysis (Incorrect Hypothesis)

**Bug**: Cannot parse otdfctl-created NanoTDF files
**Error**: `failed to fill whole buffer`
**Root Cause Hypothesis**: Resource Locator identifier bytes are read/written in wrong position

## The Bug

### Spec Requirement (from NanoTDF v1 spec)

Resource Locator binary format:
```
┌────────────────┬──────────────┬────────────┬─────────────────────┐
│ Protocol (1B)  │ Body Len (1B)│ Body (var) │ Identifier (0-32B)  │
└────────────────┴──────────────┴────────────┴─────────────────────┘
```

Protocol byte bitfield:
```
┌─────────────────┬──────────────────┐
│ID Type (4b)     │Protocol Enum (4b)│
└─────────────────┴──────────────────┘
```

**ORDER**: Protocol byte → Body length → Body bytes → Identifier bytes

### What otdfctl Creates

From hex dump of `/tmp/test-otdfctl-created.nanotdf.tdf`:

```
Offset  Bytes           Meaning
------  -----           -------
3       10              Protocol byte: ID type=0x1 (TwoByte), Protocol=0x0 (HTTP)
4       12              Body length: 18 bytes
5-22    6c6f63616c...   Body: "localhost:8080/kas" (18 bytes)
23-24   6531            Identifier: "e1" (2 bytes)
```

**✓ CORRECT**: Identifier comes AFTER body

### What Our Code Might Be Doing

Check `crates/protocol/src/nanotdf/resource_locator.rs` BinaryRead/BinaryWrite implementations.

**Possible bug**: Reading/writing identifier BEFORE body, causing misalignment

## How to Fix

### 1. Check BinaryWrite Implementation

```rust
impl BinaryWrite for ResourceLocator {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // 1. Write protocol byte (with ID type in upper nibble)
        let protocol_byte = ...; // Protocol in lower 4 bits, ID type in upper 4 bits
        write_u8(writer, protocol_byte)?;

        // 2. Write body length
        write_u8(writer, self.body.len() as u8)?;

        // 3. Write body bytes
        writer.write_all(&self.body)?;

        // 4. Write identifier bytes (if present)
        if let Some(ref id) = self.identifier {
            writer.write_all(id)?;
        }

        Ok(())
    }
}
```

### 2. Check BinaryRead Implementation

```rust
impl BinaryRead for ResourceLocator {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        // 1. Read protocol byte
        let protocol_byte = read_u8(reader)?;
        let protocol = Protocol::from_bits(protocol_byte & 0x0F)?;
        let id_type = IdentifierType::from_bits(protocol_byte)?; // Upper 4 bits

        // 2. Read body length
        let body_len = read_u8(reader)?;

        // 3. Read body bytes
        let mut body = vec![0u8; body_len as usize];
        reader.read_exact(&mut body)?;

        // 4. Read identifier bytes (if present)
        let identifier = if id_type != IdentifierType::None {
            let id_len = id_type.byte_length();
            let mut id_bytes = vec![0u8; id_len];
            reader.read_exact(&mut id_bytes)?;
            Some(id_bytes)
        } else {
            None
        };

        Ok(ResourceLocator {
            protocol,
            identifier,
            body,
        })
    }
}
```

## Test After Fix

```bash
# Should now parse successfully
cargo run --example decrypt_otdfctl_nanotdf
```

Expected output:
```
✓ Header parsed successfully
```

## Why This Breaks otdfctl Compatibility

If identifier is in wrong position:

**When we write** (Rust → otdfctl):
```
[protocol][identifier][body_len][body]  ← WRONG
```

otdfctl tries to read:
```
[protocol][body_len][body][identifier]  ← EXPECTED
```

Result: otdfctl reads identifier bytes as body length, reads wrong amount, parser fails

**When we read** (otdfctl → Rust):
```
otdfctl writes: [protocol][body_len][body][identifier]  ← CORRECT
```

We try to read:
```
[protocol][identifier][body_len][body]  ← WRONG
```

Result: We read body bytes as identifier, then try to read huge body length, "failed to fill whole buffer"

## Impact

- ❌ Cannot parse otdfctl-created files
- ❌ otdfctl cannot parse Rust-created files
- ❌ Complete binary format incompatibility
- ✅ Crypto is actually correct!
- ✅ Once fixed, otdfctl decrypt should work

## Priority

**CRITICAL - BLOCKING**

This must be fixed before any cross-platform testing can proceed.

## Files to Check

1. `crates/protocol/src/nanotdf/resource_locator.rs` - BinaryRead/BinaryWrite impls
2. Test with: `examples/decrypt_otdfctl_nanotdf.rs`
3. Verify with: `examples/nanotdf_with_kas_key.rs`

## Verification Steps

After fix:

1. Parse otdfctl file: `cargo run --example decrypt_otdfctl_nanotdf`
   → Should parse header successfully

2. Create with Rust: `cargo run --example nanotdf_with_kas_key`
   → Creates `/tmp/test-with-kas-key.nanotdf`

3. Decrypt with otdfctl:
   ```bash
   /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt \
     /tmp/test-with-kas-key.nanotdf \
     --host http://localhost:8080 --tls-no-verify \
     --with-client-creds '{"clientId":"opentdf","clientSecret":"secret"}'
   ```
   → Should decrypt successfully!

---

**Next Session**: Fix this bug first, then test cross-platform compatibility
