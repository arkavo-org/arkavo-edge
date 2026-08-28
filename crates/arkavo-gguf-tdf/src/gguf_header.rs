//! Header-only GGUF v3 parser (spec §7).
//!
//! Reads magic, version, metadata KV, and tensor infos, then computes the
//! offset of `tensor_data`. Tensor data is never touched, so a writer can plan
//! a multi-gigabyte model without loading weights.

use crate::error::GgufTdfError;
use crate::ggml_type::tensor_nbytes;
use std::io::{Read, Seek, SeekFrom};

/// Largest tensor name ggml accepts: `gguf.cpp` rejects
/// `name.length() >= GGML_MAX_NAME`, and `GGML_MAX_NAME` is 64.
pub const MAX_TENSOR_NAME_BYTES: usize = 63;

/// [GGUF] caps metadata keys at 65535 bytes. A longer string in the header is
/// a malformed or hostile file, not a model this profile should wrap.
const MAX_STRING_BYTES: u64 = 65_535;

/// Default GGUF alignment when `general.alignment` is absent.
const DEFAULT_ALIGNMENT: u64 = 32;

/// GGUF metadata value type discriminants.
const GGUF_TYPE_UINT32: u32 = 4;
const GGUF_TYPE_STRING: u32 = 8;
const GGUF_TYPE_ARRAY: u32 = 9;

/// One tensor as described by the GGUF tensor-info table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderTensor {
    pub name: String,
    /// Offset relative to `tensor_data`, as stored in the file.
    pub gguf_offset: u64,
    /// Data size in bytes, excluding trailing alignment padding.
    pub size: u64,
}

/// Everything the packer and the index binding need from a GGUF header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufHeader {
    /// `general.alignment` if present, else 32.
    pub alignment: u64,
    /// Offset of `tensor_data`; equals `gguf_get_data_offset`.
    pub data_offset: u64,
    /// Tensors in GGUF file order.
    pub tensors: Vec<HeaderTensor>,
}

/// Spec §7.3: magic, then the little-endian format version.
///
/// [GGUF] keeps the magic as `GGUF` at the byte level even for big-endian
/// files, so endianness is inferred from the version word rather than from a
/// reversed magic (which is not in [GGUF] and must not be treated as a BE
/// marker).
pub fn identify(first_eight: &[u8]) -> Result<(), GgufTdfError> {
    if first_eight.len() < 8 || first_eight[..4] != [0x47, 0x47, 0x55, 0x46] {
        return Err(GgufTdfError::NotGguf);
    }
    let version = u32::from_le_bytes([
        first_eight[4],
        first_eight[5],
        first_eight[6],
        first_eight[7],
    ]);
    match version {
        3 => Ok(()),
        1 | 2 => Err(GgufTdfError::UnsupportedGgufVersion(version)),
        // A plausible big-endian encoding of a small version number: the
        // significant bytes land in the high half of the little-endian word.
        v if v.swap_bytes() == 3 || v.trailing_zeros() >= 16 => {
            Err(GgufTdfError::UnsupportedEndian)
        }
        v => Err(GgufTdfError::UnsupportedGgufVersion(v)),
    }
}

/// Parses a GGUF header from the start of `reader`.
///
/// # Panics
///
/// Panics only if slicing the fixed 24-byte prefix into 8-byte words fails,
/// which cannot happen for a buffer of that length.
pub fn parse_header<R: Read + Seek>(reader: &mut R) -> Result<GgufHeader, GgufTdfError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut prefix = [0u8; 24];
    reader.read_exact(&mut prefix)?;
    identify(&prefix[..8])?;

    let tensor_count = u64::from_le_bytes(prefix[8..16].try_into().expect("8 bytes"));
    let kv_count = u64::from_le_bytes(prefix[16..24].try_into().expect("8 bytes"));

    let alignment = read_alignment(reader, kv_count)?;
    if alignment < 8 || !alignment.is_power_of_two() {
        return Err(GgufTdfError::BadAlign(alignment));
    }

    let tensors = read_tensor_infos(reader, tensor_count)?;

    // Padding after the tensor infos puts `tensor_data` on an ALIGN boundary.
    let end_of_infos = reader.stream_position()?;
    let data_offset = end_of_infos.div_ceil(alignment) * alignment;

    Ok(GgufHeader {
        alignment,
        data_offset,
        tensors,
    })
}

/// Walks the metadata KV block, keeping only `general.alignment`.
fn read_alignment<R: Read + Seek>(reader: &mut R, kv_count: u64) -> Result<u64, GgufTdfError> {
    let mut alignment = DEFAULT_ALIGNMENT;
    for _ in 0..kv_count {
        let key = read_gguf_string(reader)?;
        let value_type = read_u32(reader)?;
        if key == "general.alignment" {
            if value_type != GGUF_TYPE_UINT32 {
                return Err(GgufTdfError::BadHeader(
                    "general.alignment must be a uint32".to_string(),
                ));
            }
            alignment = u64::from(read_u32(reader)?);
        } else {
            skip_value(reader, value_type)?;
        }
    }
    Ok(alignment)
}

fn read_tensor_infos<R: Read + Seek>(
    reader: &mut R,
    tensor_count: u64,
) -> Result<Vec<HeaderTensor>, GgufTdfError> {
    // Cap the pre-allocation so a forged count cannot drive a huge alloc; the
    // vector still grows to whatever the file actually contains.
    let mut tensors = Vec::with_capacity(tensor_count.min(65_536) as usize);
    for _ in 0..tensor_count {
        let name = read_gguf_string(reader)?;
        if name.len() > MAX_TENSOR_NAME_BYTES {
            return Err(GgufTdfError::BadTensor(format!(
                "tensor name is {} UTF-8 bytes; ggml rejects names of 64 or more",
                name.len()
            )));
        }

        let n_dims = read_u32(reader)?;
        if n_dims == 0 || n_dims > 4 {
            return Err(GgufTdfError::BadTensor(format!(
                "n_dimensions is {n_dims}, expected 1..=4"
            )));
        }
        let mut ne = [1u64; 4];
        for slot in ne.iter_mut().take(n_dims as usize) {
            *slot = read_u64(reader)?;
        }

        let ggml_type = read_u32(reader)?;
        let gguf_offset = read_u64(reader)?;

        tensors.push(HeaderTensor {
            name,
            gguf_offset,
            size: tensor_nbytes(ggml_type, &ne)?,
        });
    }
    Ok(tensors)
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32, GgufTdfError> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(reader: &mut R) -> Result<u64, GgufTdfError> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

/// GGUF strings are UTF-8, length-prepended with a `u64`, not NUL-terminated.
fn read_gguf_string<R: Read>(reader: &mut R) -> Result<String, GgufTdfError> {
    let len = read_u64(reader)?;
    if len > MAX_STRING_BYTES {
        return Err(GgufTdfError::BadHeader(format!(
            "GGUF string of {len} bytes exceeds the 65535-byte limit"
        )));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| GgufTdfError::BadHeader(format!("invalid UTF-8: {e}")))
}

/// Advances past one metadata value without materializing it.
fn skip_value<R: Read + Seek>(reader: &mut R, value_type: u32) -> Result<(), GgufTdfError> {
    let width = match value_type {
        0 | 1 | 7 => 1,        // uint8, int8, bool
        2 | 3 => 2,            // uint16, int16
        4..=6 => 4,            // uint32, int32, float32
        10..=12 => 8,          // uint64, int64, float64
        GGUF_TYPE_STRING => 0, // handled below
        GGUF_TYPE_ARRAY => {
            let elem_type = read_u32(reader)?;
            let count = read_u64(reader)?;
            if elem_type == GGUF_TYPE_ARRAY {
                return Err(GgufTdfError::BadHeader(
                    "nested GGUF arrays are not supported".to_string(),
                ));
            }
            for _ in 0..count {
                skip_value(reader, elem_type)?;
            }
            return Ok(());
        }
        other => {
            return Err(GgufTdfError::BadHeader(format!(
                "unknown GGUF metadata value type {other}"
            )));
        }
    };

    if value_type == GGUF_TYPE_STRING {
        let len = read_u64(reader)?;
        if len > MAX_STRING_BYTES {
            return Err(GgufTdfError::BadHeader(format!(
                "GGUF string of {len} bytes exceeds the 65535-byte limit"
            )));
        }
        // `len` is bounded by MAX_STRING_BYTES, so the cast cannot wrap.
        reader.seek(SeekFrom::Current(len.cast_signed()))?;
    } else {
        reader.seek(SeekFrom::Current(width))?;
    }
    Ok(())
}
