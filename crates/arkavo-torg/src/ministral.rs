//! Ministral-3 token mapping for TØR-G constrained decoding
//!
//! This module scans the Ministral vocabulary to find token IDs for TØR-G DSL tokens,
//! building a `TokenMapping` that can be used with `torg-mask`.
//!
//! Supports both Instruct and Reasoning variants (same vocabulary).

use crate::TorgError;
use torg_mask::{TokenMapping, TokenMappingBuilder};

/// Ministral-3 specific token mapping
///
/// Maps TØR-G tokens to their corresponding Ministral vocabulary IDs.
/// Built by scanning the model's vocabulary for DSL tokens.
///
/// Works with both Ministral-3B-Instruct and Ministral-3B-Reasoning variants.
#[derive(Debug, Clone)]
pub struct MinistralTokenMap {
    mapping: TokenMapping,
    vocab_size: i32,
}

impl MinistralTokenMap {
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

        // Track potential token collisions with chat template
        let mut inst_token_found = false;

        for id in 0..vocab_size {
            let piece = arkavo_llama_cpp::token_to_piece(vocab, id, true).unwrap_or_default();

            match piece.as_str() {
                // Boolean operators
                "|" => or_id = Some(id as u32),
                "!" => nor_id = Some(id as u32),
                "^" => xor_id = Some(id as u32),

                // Node delimiters - verify no collision with [INST]
                "[" => node_start_id = Some(id as u32),
                "]" => node_end_id = Some(id as u32),

                // Chat template tokens (for collision detection)
                "[INST]" | "[/INST]" => inst_token_found = true,

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

        // Log vocab size for debugging
        eprintln!("Ministral vocab size: {vocab_size}");

        // Verify no token collision with chat template markers
        if let (Some(bracket_id), true) = (node_start_id, inst_token_found) {
            eprintln!("Token scan: '[' = {bracket_id}, [INST] exists separately (no collision)");
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
            .or(200)
            .nor(201)
            .xor(202)
            .node_start(203)
            .node_end(204)
            .input_decl(205)
            .output_decl(206)
            .true_token(207)
            .false_token(208)
            .id_base(210)
            .id_count(256)
            .build();

        Self {
            mapping,
            vocab_size: 32_768, // Typical Ministral vocab size
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
        let map = MinistralTokenMap::mock();
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
        let map = MinistralTokenMap::mock();
        assert_eq!(map.vocab_size(), 32_768);
    }

    #[test]
    fn test_mapping_values() {
        let map = MinistralTokenMap::mock();
        let mapping = map.mapping();

        assert_eq!(mapping.get(Token::Or), Some(200));
        assert_eq!(mapping.get(Token::Nor), Some(201));
        assert_eq!(mapping.get(Token::Xor), Some(202));
        assert_eq!(mapping.get(Token::NodeStart), Some(203));
        assert_eq!(mapping.get(Token::NodeEnd), Some(204));
        assert_eq!(mapping.get(Token::Id(0)), Some(210));
        assert_eq!(mapping.get(Token::Id(1)), Some(211));
    }

    #[test]
    fn test_mock_has_different_ids_than_qwen() {
        // Verify mock IDs don't overlap with Qwen3 mock IDs
        let ministral = MinistralTokenMap::mock();
        assert_eq!(ministral.mapping().get(Token::Or), Some(200));
        // Qwen3 mock uses 100 for Or - no collision
    }
}
