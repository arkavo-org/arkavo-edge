//! Minimal GGUF header parser.
//!
//! Reads only the metadata block of a GGUF file — magic, version, counts and
//! key/value pairs — so a model can be identified without loading its weights.
//! Array values are skipped via `Seek` to keep parsing bounded for files whose
//! headers embed large tokenizer tables.

use crate::{AttestationError, Result};
use std::io::{Read, Seek, SeekFrom};

const GGUF_MAGIC: u32 = 0x4655_4747; // "GGUF" little-endian

/// Upper bound on an individual GGUF string. Keys and `general.*` values are
/// tiny; the cap exists only to reject a malformed length before allocation.
const MAX_STRING_LEN: u64 = 64 * 1024 * 1024;

/// GGUF metadata value type tags, per the GGUF specification.
mod value_type {
    pub const UINT8: u32 = 0;
    pub const INT8: u32 = 1;
    pub const UINT16: u32 = 2;
    pub const INT16: u32 = 3;
    pub const UINT32: u32 = 4;
    pub const INT32: u32 = 5;
    pub const FLOAT32: u32 = 6;
    pub const BOOL: u32 = 7;
    pub const STRING: u32 = 8;
    pub const ARRAY: u32 = 9;
    pub const UINT64: u32 = 10;
    pub const INT64: u32 = 11;
    pub const FLOAT64: u32 = 12;
}

/// Identifying metadata extracted from a GGUF file header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GgufMetadata {
    pub version: u32,
    pub tensor_count: u64,
    pub architecture: Option<String>,
    pub name: Option<String>,
    pub file_type: Option<u32>,
}

impl GgufMetadata {
    /// Human-readable quantization label derived from `general.file_type`.
    pub fn quantization(&self) -> Option<String> {
        self.file_type.map(file_type_name)
    }
}

/// Parse the metadata block of a GGUF stream positioned at its start.
pub fn parse<R: Read + Seek>(reader: &mut R) -> Result<GgufMetadata> {
    if read_u32(reader)? != GGUF_MAGIC {
        return Err(AttestationError::Model("not a GGUF file".to_string()));
    }
    let version = read_u32(reader)?;
    if !(2..=3).contains(&version) {
        return Err(AttestationError::Model(format!(
            "unsupported GGUF version {version}"
        )));
    }
    let tensor_count = read_u64(reader)?;
    let kv_count = read_u64(reader)?;

    let mut meta = GgufMetadata {
        version,
        tensor_count,
        ..GgufMetadata::default()
    };
    for _ in 0..kv_count {
        let key = read_gguf_string(reader)?;
        let vt = read_u32(reader)?;
        match key.as_str() {
            "general.architecture" => meta.architecture = Some(expect_string(reader, vt, &key)?),
            "general.name" => meta.name = Some(expect_string(reader, vt, &key)?),
            "general.file_type" => meta.file_type = Some(expect_u32(reader, vt, &key)?),
            _ => skip_value(reader, vt)?,
        }
    }
    Ok(meta)
}

/// Map a `general.file_type` (`llama_ftype`) enum value to a quantization name.
fn file_type_name(ft: u32) -> String {
    let name = match ft {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        _ => return format!("ftype-{ft}"),
    };
    name.to_string()
}

fn io_err(e: std::io::Error) -> AttestationError {
    AttestationError::Model(format!("malformed GGUF header: {e}"))
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf).map_err(io_err)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(reader: &mut R) -> Result<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf).map_err(io_err)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_gguf_string<R: Read>(reader: &mut R) -> Result<String> {
    let len = read_u64(reader)?;
    if len > MAX_STRING_LEN {
        return Err(AttestationError::Model(format!(
            "GGUF string length {len} exceeds limit"
        )));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).map_err(io_err)?;
    String::from_utf8(buf)
        .map_err(|e| AttestationError::Model(format!("GGUF string is not UTF-8: {e}")))
}

fn expect_string<R: Read>(reader: &mut R, vt: u32, key: &str) -> Result<String> {
    if vt != value_type::STRING {
        return Err(AttestationError::Model(format!(
            "GGUF key '{key}' expected string, got value type {vt}"
        )));
    }
    read_gguf_string(reader)
}

fn expect_u32<R: Read>(reader: &mut R, vt: u32, key: &str) -> Result<u32> {
    match vt {
        value_type::UINT32 | value_type::INT32 => read_u32(reader),
        _ => Err(AttestationError::Model(format!(
            "GGUF key '{key}' expected uint32, got value type {vt}"
        ))),
    }
}

/// Byte width of a fixed-size scalar value type, or `None` for variable types.
fn scalar_size(vt: u32) -> Option<u64> {
    Some(match vt {
        value_type::UINT8 | value_type::INT8 | value_type::BOOL => 1,
        value_type::UINT16 | value_type::INT16 => 2,
        value_type::UINT32 | value_type::INT32 | value_type::FLOAT32 => 4,
        value_type::UINT64 | value_type::INT64 | value_type::FLOAT64 => 8,
        _ => return None,
    })
}

fn seek_forward<R: Seek>(reader: &mut R, n: u64) -> Result<()> {
    let offset =
        i64::try_from(n).map_err(|_| AttestationError::Model("GGUF offset too large".into()))?;
    reader.seek(SeekFrom::Current(offset)).map_err(io_err)?;
    Ok(())
}

/// Advance past a value of `vt` without interpreting it.
fn skip_value<R: Read + Seek>(reader: &mut R, vt: u32) -> Result<()> {
    if let Some(sz) = scalar_size(vt) {
        return seek_forward(reader, sz);
    }
    match vt {
        value_type::STRING => {
            let len = read_u64(reader)?;
            seek_forward(reader, len)
        }
        value_type::ARRAY => {
            let elem_type = read_u32(reader)?;
            let count = read_u64(reader)?;
            if let Some(sz) = scalar_size(elem_type) {
                let total = count
                    .checked_mul(sz)
                    .ok_or_else(|| AttestationError::Model("GGUF array size overflow".into()))?;
                seek_forward(reader, total)
            } else if elem_type == value_type::STRING {
                for _ in 0..count {
                    let len = read_u64(reader)?;
                    seek_forward(reader, len)?;
                }
                Ok(())
            } else {
                Err(AttestationError::Model(format!(
                    "unsupported GGUF array element type {elem_type}"
                )))
            }
        }
        _ => Err(AttestationError::Model(format!(
            "unknown GGUF value type {vt}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn put_str(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }

    /// A GGUF v3 header exercising scalar, string, scalar-array and
    /// string-array value types so the skip path is covered.
    fn synthetic_gguf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes()); // version
        b.extend_from_slice(&291u64.to_le_bytes()); // tensor_count
        b.extend_from_slice(&5u64.to_le_bytes()); // kv_count

        put_str(&mut b, "general.architecture");
        b.extend_from_slice(&value_type::STRING.to_le_bytes());
        put_str(&mut b, "llama");

        put_str(&mut b, "general.name");
        b.extend_from_slice(&value_type::STRING.to_le_bytes());
        put_str(&mut b, "Test Model");

        put_str(&mut b, "general.file_type");
        b.extend_from_slice(&value_type::UINT32.to_le_bytes());
        b.extend_from_slice(&15u32.to_le_bytes());

        put_str(&mut b, "llama.scalar_array");
        b.extend_from_slice(&value_type::ARRAY.to_le_bytes());
        b.extend_from_slice(&value_type::UINT32.to_le_bytes());
        b.extend_from_slice(&3u64.to_le_bytes());
        b.extend_from_slice(&[0u8; 12]);

        put_str(&mut b, "tokenizer.ggml.tokens");
        b.extend_from_slice(&value_type::ARRAY.to_le_bytes());
        b.extend_from_slice(&value_type::STRING.to_le_bytes());
        b.extend_from_slice(&2u64.to_le_bytes());
        put_str(&mut b, "a");
        put_str(&mut b, "bb");
        b
    }

    #[test]
    fn parses_general_metadata() {
        let meta = parse(&mut Cursor::new(synthetic_gguf())).expect("parse");
        assert_eq!(meta.version, 3);
        assert_eq!(meta.tensor_count, 291);
        assert_eq!(meta.architecture.as_deref(), Some("llama"));
        assert_eq!(meta.name.as_deref(), Some("Test Model"));
        assert_eq!(meta.file_type, Some(15));
        assert_eq!(meta.quantization().as_deref(), Some("Q4_K_M"));
    }

    #[test]
    fn rejects_bad_magic() {
        let err = parse(&mut Cursor::new(b"NOPE____".to_vec())).unwrap_err();
        assert!(err.to_string().contains("not a GGUF"));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut b = Vec::new();
        b.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[0u8; 16]);
        let err = parse(&mut Cursor::new(b)).unwrap_err();
        assert!(err.to_string().contains("unsupported GGUF version 1"));
    }

    #[test]
    fn rejects_truncated_header() {
        let mut b = Vec::new();
        b.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes());
        let err = parse(&mut Cursor::new(b)).unwrap_err();
        assert!(err.to_string().contains("malformed GGUF header"));
    }

    #[test]
    fn rejects_oversized_string_length() {
        let mut b = Vec::new();
        b.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        b.extend_from_slice(&1u64.to_le_bytes()); // kv_count
        b.extend_from_slice(&(MAX_STRING_LEN + 1).to_le_bytes()); // bogus key length
        let err = parse(&mut Cursor::new(b)).unwrap_err();
        assert!(err.to_string().contains("exceeds limit"));
    }

    #[test]
    fn unknown_file_type_falls_back() {
        assert_eq!(file_type_name(9999), "ftype-9999");
    }
}
