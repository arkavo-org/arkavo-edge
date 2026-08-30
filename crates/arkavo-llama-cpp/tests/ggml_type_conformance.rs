//! Pins `arkavo-gguf-tdf`'s transcribed quantization table against ggml.
//!
//! That table is hand-written so the GGUF packer works on targets where
//! llama-cpp is not built. Wherever llama-cpp *is* available, every entry must
//! agree with ggml itself: a wrong `type_size` silently mispacks tensor
//! extents, and the mistake would only surface as corrupt weights.

#![cfg(not(target_env = "musl"))]

use arkavo_gguf_tdf::block_traits;
use arkavo_llama_cpp::ffi;

/// One past the last `ggml_type` discriminant (`GGML_TYPE_COUNT`).
const GGML_TYPE_COUNT: u32 = 43;

#[test]
fn transcribed_quant_table_matches_ggml() {
    let mut checked = 0usize;

    for type_id in 0..GGML_TYPE_COUNT {
        // SAFETY: `type_id` is within the ggml_type enum's declared range.
        let (ggml_blck, ggml_size) = unsafe {
            (
                ffi::ggml_blck_size(type_id as ffi::ggml_type),
                ffi::ggml_type_size(type_id as ffi::ggml_type),
            )
        };

        match block_traits(type_id) {
            Some((blck, size)) => {
                assert_eq!(
                    blck, ggml_blck as u64,
                    "block size mismatch for ggml_type {type_id}"
                );
                assert_eq!(
                    size, ggml_size as u64,
                    "type size mismatch for ggml_type {type_id}"
                );
                checked += 1;
            }
            None => {
                // A type the table omits must be one ggml itself has removed,
                // which leaves a zeroed slot in its trait table.
                assert_eq!(
                    ggml_blck, 0,
                    "ggml_type {type_id} is live in ggml but missing from the table"
                );
            }
        }
    }

    assert!(
        checked >= 30,
        "expected to verify the bulk of the table, checked only {checked}"
    );
}
