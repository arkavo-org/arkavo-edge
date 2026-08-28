//! Synthetic GGUF v3 builder shared by the conformance tests.
//!
//! Hand-rolled so the tests never depend on llama.cpp being built, and so a
//! test can deliberately produce a malformed file.

#![allow(dead_code, unreachable_pub, clippy::missing_panics_doc)]

use arkavo_gguf_tdf::tensor_nbytes;

/// GGUF metadata value type discriminants used by this fixture.
const GGUF_TYPE_UINT32: u32 = 4;
const GGUF_TYPE_STRING: u32 = 8;

pub fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u64).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// A tensor to place in a synthetic file: `(name, ggml_type, ne)`.
pub type FixtureTensor<'a> = (&'a str, u32, [u64; 4]);

/// Where each tensor lands, relative to `tensor_data`.
pub fn tensor_layout(tensors: &[FixtureTensor], align: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::with_capacity(tensors.len());
    let mut offset = 0u64;
    for (_, ty, ne) in tensors {
        let size = tensor_nbytes(*ty, ne).expect("fixture tensor must be valid");
        out.push((offset, size));
        offset = (offset + size).div_ceil(align) * align;
    }
    out
}

/// Builds a little-endian GGUF v3 file.
///
/// `alignment` writes a `general.alignment` uint32 KV when `Some`; when
/// `None` the file relies on the default of 32. Tensor data is a deterministic
/// per-tensor byte pattern so a reader can prove it served the right bytes.
pub fn synthetic_gguf(tensors: &[FixtureTensor], alignment: Option<u32>) -> Vec<u8> {
    let align = u64::from(alignment.unwrap_or(32));

    let mut kv = Vec::new();
    let mut kv_count = 0u64;
    if let Some(a) = alignment {
        put_str(&mut kv, "general.alignment");
        kv.extend_from_slice(&GGUF_TYPE_UINT32.to_le_bytes());
        kv.extend_from_slice(&a.to_le_bytes());
        kv_count += 1;
    }
    put_str(&mut kv, "general.architecture");
    kv.extend_from_slice(&GGUF_TYPE_STRING.to_le_bytes());
    put_str(&mut kv, "llama");
    kv_count += 1;

    let layout = tensor_layout(tensors, align);

    let mut infos = Vec::new();
    for ((name, ty, ne), (offset, _)) in tensors.iter().zip(&layout) {
        let n_dims = ne.iter().rposition(|d| *d > 1).map_or(1, |i| i + 1) as u32;
        put_str(&mut infos, name);
        infos.extend_from_slice(&n_dims.to_le_bytes());
        for d in ne.iter().take(n_dims as usize) {
            infos.extend_from_slice(&d.to_le_bytes());
        }
        infos.extend_from_slice(&ty.to_le_bytes());
        infos.extend_from_slice(&offset.to_le_bytes());
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&(tensors.len() as u64).to_le_bytes());
    out.extend_from_slice(&kv_count.to_le_bytes());
    out.extend_from_slice(&kv);
    out.extend_from_slice(&infos);
    while !(out.len() as u64).is_multiple_of(align) {
        out.push(0);
    }

    let data_offset = out.len() as u64;
    for (i, (offset, size)) in layout.iter().enumerate() {
        let start = (data_offset + offset) as usize;
        // Alignment padding between tensors stays zero, as [GGUF] requires.
        out.resize(start, 0);
        out.extend((0..*size).map(|b| (b as u8) ^ (i as u8).wrapping_mul(31).wrapping_add(1)));
    }
    out
}

/// Writes a synthetic GGUF to `dir` and returns its path.
pub fn write_gguf(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}
