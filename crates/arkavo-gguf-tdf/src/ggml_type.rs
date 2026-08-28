//! ggml quantization block traits.
//!
//! Values transcribed from vendored llama.cpp `gguf-py/gguf/constants.py`
//! (`GGML_QUANT_SIZES`, `QK_K = 256`) with discriminants from the `ggml_type`
//! enum in `ggml/include/ggml.h`.
//!
//! This is a pure-Rust table rather than an FFI call so the writer works on
//! targets where llama-cpp is not built (Windows, musl). Wherever llama-cpp is
//! available, `tests/ggml_conformance.rs` pins every entry against
//! `ggml_blck_size` / `ggml_type_size`.

use crate::error::GgufTdfError;

/// `(blck_size, type_size)` for a `ggml_type` discriminant.
///
/// `None` for discriminants ggml has removed (Q4_2, Q4_3, the Q4_0_N_M
/// repacks, the IQ4_NL_N_M repacks) and for values past `GGML_TYPE_COUNT`.
pub fn block_traits(ggml_type: u32) -> Option<(u64, u64)> {
    const QK_K: u64 = 256;
    let traits = match ggml_type {
        0 => (1, 4),                                           // F32
        1 => (1, 2),                                           // F16
        2 => (32, 2 + 16),                                     // Q4_0
        3 => (32, 2 + 2 + 16),                                 // Q4_1
        6 => (32, 2 + 4 + 16),                                 // Q5_0
        7 => (32, 2 + 2 + 4 + 16),                             // Q5_1
        8 => (32, 2 + 32),                                     // Q8_0
        9 => (32, 4 + 4 + 32),                                 // Q8_1
        10 => (QK_K, 2 + 2 + QK_K / 16 + QK_K / 4),            // Q2_K
        11 => (QK_K, 2 + QK_K / 4 + QK_K / 8 + 12),            // Q3_K
        12 => (QK_K, 2 + 2 + QK_K / 2 + 12),                   // Q4_K
        13 => (QK_K, 2 + 2 + QK_K / 2 + QK_K / 8 + 12),        // Q5_K
        14 => (QK_K, 2 + QK_K / 2 + QK_K / 4 + QK_K / 16),     // Q6_K
        15 => (QK_K, 4 + QK_K + QK_K / 8),                     // Q8_K
        16 => (QK_K, 2 + QK_K / 4),                            // IQ2_XXS
        17 => (QK_K, 2 + QK_K / 4 + QK_K / 32),                // IQ2_XS
        18 => (QK_K, 2 + QK_K / 4 + QK_K / 8),                 // IQ3_XXS
        19 => (QK_K, 2 + QK_K / 8 + QK_K / 16),                // IQ1_S
        20 => (32, 2 + 16),                                    // IQ4_NL
        21 => (QK_K, 2 + QK_K / 4 + QK_K / 8 + QK_K / 32 + 4), // IQ3_S
        22 => (QK_K, 2 + QK_K / 4 + QK_K / 16),                // IQ2_S
        23 => (QK_K, 2 + 2 + QK_K / 2 + QK_K / 64),            // IQ4_XS
        24 => (1, 1),                                          // I8
        25 => (1, 2),                                          // I16
        26 => (1, 4),                                          // I32
        27 => (1, 8),                                          // I64
        28 => (1, 8),                                          // F64
        29 => (QK_K, QK_K / 8 + QK_K / 16 + QK_K / 32),        // IQ1_M
        30 => (1, 2),                                          // BF16
        34 => (QK_K, 2 + 4 * 13),                              // TQ1_0
        35 => (QK_K, 2 + 64),                                  // TQ2_0
        39 => (32, 1 + 16),                                    // MXFP4
        40 => (64, 4 + 32),                                    // NVFP4
        41 => (128, 2 + 16),                                   // Q1_0
        42 => (64, 2 + 16),                                    // Q2_0
        _ => return None,
    };
    Some(traits)
}

/// Bytes a tensor's data occupies, excluding trailing alignment padding.
///
/// This mirrors `ggml_nbytes` for the contiguous tensors a GGUF file holds:
/// one row is `type_size * ne[0] / blck_size`, repeated across the higher
/// dimensions.
pub fn tensor_nbytes(ggml_type: u32, ne: &[u64; 4]) -> Result<u64, GgufTdfError> {
    let (blck_size, type_size) = block_traits(ggml_type)
        .ok_or_else(|| GgufTdfError::BadTensor(format!("unknown ggml_type {ggml_type}")))?;

    if !ne[0].is_multiple_of(blck_size) {
        return Err(GgufTdfError::BadTensor(format!(
            "ne[0]={} is not a multiple of block size {blck_size} for ggml_type {ggml_type}",
            ne[0]
        )));
    }

    let row = (ne[0] / blck_size)
        .checked_mul(type_size)
        .ok_or_else(|| GgufTdfError::BadTensor("row size overflow".to_string()))?;

    ne[1..]
        .iter()
        .try_fold(row, |acc, d| acc.checked_mul(*d))
        .ok_or_else(|| GgufTdfError::BadTensor("tensor size overflow".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_block_traits_match_ggml() {
        assert_eq!(block_traits(0), Some((1, 4))); // F32
        assert_eq!(block_traits(1), Some((1, 2))); // F16
        assert_eq!(block_traits(2), Some((32, 18))); // Q4_0
        assert_eq!(block_traits(8), Some((32, 34))); // Q8_0
        assert_eq!(block_traits(12), Some((256, 144))); // Q4_K
        assert_eq!(block_traits(14), Some((256, 210))); // Q6_K
        assert_eq!(block_traits(15), Some((256, 292))); // Q8_K
        assert_eq!(block_traits(29), Some((256, 56))); // IQ1_M
        assert_eq!(block_traits(30), Some((1, 2))); // BF16
        assert_eq!(block_traits(4), None, "Q4_2 was removed from ggml");
        assert_eq!(block_traits(31), None, "Q4_0_4_4 was removed from ggml");
        assert_eq!(block_traits(43), None, "GGML_TYPE_COUNT is not a type");
    }

    #[test]
    fn nbytes_matches_row_major_formula() {
        assert_eq!(tensor_nbytes(0, &[4096, 2, 1, 1]).unwrap(), 4096 * 2 * 4);
        assert_eq!(
            tensor_nbytes(12, &[4096, 1, 1, 1]).unwrap(),
            (4096 / 256) * 144
        );
        assert_eq!(
            tensor_nbytes(14, &[8192, 3, 1, 1]).unwrap(),
            (8192 / 256) * 210 * 3
        );
        // Higher dimensions multiply through.
        assert_eq!(
            tensor_nbytes(1, &[64, 2, 3, 4]).unwrap(),
            64 * 2 * 3 * 4 * 2
        );
    }

    #[test]
    fn nbytes_rejects_rows_that_do_not_fill_whole_blocks() {
        let err = tensor_nbytes(12, &[100, 1, 1, 1]).unwrap_err();
        assert_eq!(err.code(), "GGUFTDF_BAD_TENSOR");
    }

    #[test]
    fn nbytes_rejects_unknown_type() {
        let err = tensor_nbytes(4, &[32, 1, 1, 1]).unwrap_err();
        assert_eq!(err.code(), "GGUFTDF_BAD_TENSOR");
    }

    #[test]
    fn nbytes_rejects_overflow_instead_of_wrapping() {
        let err = tensor_nbytes(0, &[u64::MAX / 2, 4, 1, 1]).unwrap_err();
        assert_eq!(err.code(), "GGUFTDF_BAD_TENSOR");
    }
}
