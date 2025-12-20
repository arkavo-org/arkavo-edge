//! Qwen3-0.6B token mapping for TØR-G constrained decoding
//!
//! This module scans the Qwen3 vocabulary to find token IDs for TØR-G DSL tokens,
//! building a `TokenMapping` that can be used with `torg-mask`.

use crate::TorgError;
use torg_mask::{TokenMapping, TokenMappingBuilder};

/// Qwen3-0.6B specific token mapping
///
/// Maps TØR-G tokens to their corresponding Qwen3 vocabulary IDs.
/// Built by scanning the model's vocabulary for DSL tokens.
#[derive(Debug, Clone)]
pub struct Qwen3TokenMap {
    mapping: TokenMapping,
    vocab_size: i32,
}

impl Qwen3TokenMap {
    /// Build token mapping by scanning a llama.cpp vocabulary
    ///
    /// # Safety
    ///
    /// The `vocab` pointer must be valid and point to an initialized llama_vocab.
    #[cfg(not(target_env = "musl"))]
    pub unsafe fn from_vocab(
        vocab: *const arkavo_llama_cpp::ffi::llama_vocab,
    ) -> Result<Self, TorgError> {
        use arkavo_llama_cpp::ffi;

        // SAFETY: caller guarantees vocab is valid
        let vocab_size = unsafe { ffi::llama_vocab_n_tokens(vocab) };

        let mut or_id = None;
        let mut nor_id = None;
        let mut xor_id = None;
        let mut node_start_id = None;
        let mut node_end_id = None;
        let mut input_decl_id = None;
        let mut output_decl_id = None;
        let mut true_id = None;
        let mut false_id = None;

        // Track digit tokens for Id base detection
        let mut digit_zero_id = None;

        for id in 0..vocab_size {
            let piece = arkavo_llama_cpp::token_to_piece(vocab, id, true).unwrap_or_default();

            match piece.as_str() {
                // Boolean operators
                "|" => or_id = Some(id as u32),
                "!" => nor_id = Some(id as u32),
                "^" => xor_id = Some(id as u32),

                // Node delimiters
                "[" => node_start_id = Some(id as u32),
                "]" => node_end_id = Some(id as u32),

                // Digit 0 - used as base for Id tokens
                "0" => digit_zero_id = Some(id as u32),

                // Boolean literals
                "True" | "true" | "TRUE" if true_id.is_none() => true_id = Some(id as u32),
                "False" | "false" | "FALSE" if false_id.is_none() => false_id = Some(id as u32),

                // Structural tokens using common ASCII characters
                // Use '<' for input declaration, '>' for output declaration
                "<" => input_decl_id = Some(id as u32),
                ">" => output_decl_id = Some(id as u32),

                _ => {}
            }
        }

        // All tokens must exist in the vocabulary
        let or_id =
            or_id.ok_or_else(|| TorgError::TokenMapping("Or token '|' not found".into()))?;
        let nor_id =
            nor_id.ok_or_else(|| TorgError::TokenMapping("Nor token '!' not found".into()))?;
        let xor_id =
            xor_id.ok_or_else(|| TorgError::TokenMapping("Xor token '^' not found".into()))?;
        let node_start_id = node_start_id
            .ok_or_else(|| TorgError::TokenMapping("NodeStart token '[' not found".into()))?;
        let node_end_id = node_end_id
            .ok_or_else(|| TorgError::TokenMapping("NodeEnd token ']' not found".into()))?;
        let input_decl_id = input_decl_id
            .ok_or_else(|| TorgError::TokenMapping("InputDecl token '<' not found".into()))?;
        let output_decl_id = output_decl_id
            .ok_or_else(|| TorgError::TokenMapping("OutputDecl token '>' not found".into()))?;

        // Use fallback values for boolean literals (less critical)
        let true_id = true_id.unwrap_or(vocab_size as u32 - 3);
        let false_id = false_id.unwrap_or(vocab_size as u32 - 2);

        // Use digit_zero_id as base for Id tokens, with consecutive IDs
        let id_base = digit_zero_id
            .ok_or_else(|| TorgError::TokenMapping("Digit '0' token not found".into()))?;

        let mapping = TokenMappingBuilder::new()
            .or(or_id)
            .nor(nor_id)
            .xor(xor_id)
            .node_start(node_start_id)
            .node_end(node_end_id)
            .input_decl(input_decl_id)
            .output_decl(output_decl_id)
            .true_token(true_id)
            .false_token(false_id)
            .id_base(id_base)
            .id_count(256) // Support up to 256 node IDs
            .build();

        Ok(Self {
            mapping,
            vocab_size,
        })
    }

    /// Create a token mapping for testing without a real vocabulary
    #[cfg(test)]
    pub fn mock() -> Self {
        let mapping = TokenMappingBuilder::new()
            .or(100)
            .nor(101)
            .xor(102)
            .node_start(103)
            .node_end(104)
            .input_decl(105)
            .output_decl(106)
            .true_token(107)
            .false_token(108)
            .id_base(110)
            .id_count(256)
            .build();

        Self {
            mapping,
            vocab_size: 151_936, // Qwen3-0.6B vocab size
        }
    }

    /// Get the vocabulary size
    pub fn vocab_size(&self) -> i32 {
        self.vocab_size
    }

    /// Get the underlying token mapping
    pub fn into_mapping(self) -> TokenMapping {
        self.mapping
    }

    /// Get a reference to the token mapping
    pub fn mapping(&self) -> &TokenMapping {
        &self.mapping
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::Token;

    #[test]
    fn test_mock_mapping() {
        let map = Qwen3TokenMap::mock();
        let mapping = map.mapping();

        assert!(mapping.get(Token::Or).is_some());
        assert!(mapping.get(Token::Nor).is_some());
        assert!(mapping.get(Token::Xor).is_some());
        assert!(mapping.get(Token::NodeStart).is_some());
        assert!(mapping.get(Token::NodeEnd).is_some());
        assert!(mapping.get(Token::Id(0)).is_some());
        assert!(mapping.get(Token::Id(9)).is_some());
    }

    #[test]
    fn test_vocab_size() {
        let map = Qwen3TokenMap::mock();
        assert_eq!(map.vocab_size(), 151_936);
    }

    #[test]
    fn test_mapping_values() {
        let map = Qwen3TokenMap::mock();
        let mapping = map.mapping();

        assert_eq!(mapping.get(Token::Or), Some(100));
        assert_eq!(mapping.get(Token::Nor), Some(101));
        assert_eq!(mapping.get(Token::Xor), Some(102));
        assert_eq!(mapping.get(Token::NodeStart), Some(103));
        assert_eq!(mapping.get(Token::NodeEnd), Some(104));
        assert_eq!(mapping.get(Token::Id(0)), Some(110));
        assert_eq!(mapping.get(Token::Id(1)), Some(111));
    }
}
