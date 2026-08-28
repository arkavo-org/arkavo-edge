//! `gguf-tdf/1` — Arkavo profile of OpenTDF zip TDF for GGUF model files.
//!
//! Spec: `specifications/gguf-tdf/draft-arkavo-gguf-tdf-00.md`.
//!
//! At rest the artifact is ciphertext plus a plaintext manifest and index. At
//! load, extra plaintext is bounded by `headerBytes + maxSegment`: one
//! retained GGUF header plus one cached weight segment. The executor sees a
//! virtual linear GGUF through `read_at` and contains no TDF, AES, or KAS code.

mod error;
mod ggml_type;
mod gguf_header;
mod index;
mod key;
mod pack;
mod read_at;
mod reader;
mod writer;

pub use error::GgufTdfError;
pub use ggml_type::{block_traits, tensor_nbytes};
pub use gguf_header::{GgufHeader, HeaderTensor, MAX_TENSOR_NAME_BYTES, identify, parse_header};
pub use index::{SegmentMap, build_index, validate_index};
#[cfg(feature = "kas")]
pub use key::RsaOaepWrapper;
pub use key::{PayloadKeyUnwrapper, PayloadKeyWrapper, PreResolvedKey, WrappedKey};
pub use pack::{PlannedSegment, plan_segments};
pub use read_at::VirtualGguf;
pub use reader::GgufTdfArchive;
pub use writer::{ProtectOptions, ProtectReport, protect};

/// Profile identifier carried in `manifest.gguf.profile`.
pub const PROFILE: &str = "gguf-tdf/1";

/// Default maximum plaintext size of a non-header segment (4 MiB).
pub const DEFAULT_MAX_SEGMENT: u64 = 4_194_304;

/// AES-GCM per-segment overhead: a 12-byte IV plus a 16-byte tag.
pub const SEGMENT_OVERHEAD: u64 = 28;

/// Zip member holding encrypted segment 0, the GGUF header.
pub const HEADER_ENTRY: &str = "header";

/// Manifest member this profile writes.
pub const MANIFEST_ENTRY: &str = "0.manifest.json";

/// Manifest member readers accept as a fallback.
pub const MANIFEST_ENTRY_FALLBACK: &str = "manifest.json";

/// File extension identifying a protected model.
pub const EXTENSION: &str = ".gguf.tdf";

/// Zip member name for weight segment `id`.
pub fn entry_name(id: u64) -> String {
    if id == 0 {
        HEADER_ENTRY.to_string()
    } else {
        format!("s/{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_match_the_profile_grammar() {
        assert_eq!(entry_name(0), "header");
        assert_eq!(entry_name(1), "s/1");
        assert_eq!(entry_name(4095), "s/4095");
    }
}
