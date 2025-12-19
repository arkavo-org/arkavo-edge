//! Bridge between torg-mask and llama.cpp for constrained decoding
//!
//! This module provides `TorgLlamaSampler` which wraps `torg-mask::ConstrainedDecoder`
//! and produces `llama_logit_bias` arrays for use with llama.cpp's sampler chain.

use crate::TorgError;
use torg_core::Graph;
use torg_mask::{ConstrainedDecoder, MaskGenerator, TokenMapping};

/// Constrained sampler that bridges torg-mask to llama.cpp
///
/// Maintains a `ConstrainedDecoder` and converts its `LogitMask` output
/// to llama.cpp's `llama_logit_bias` format for integration with the
/// sampler chain.
pub struct TorgLlamaSampler {
    decoder: ConstrainedDecoder,
    vocab_size: i32,
}

impl TorgLlamaSampler {
    /// Create a new TØR-G constrained sampler
    ///
    /// # Arguments
    ///
    /// * `mapping` - Token mapping from TØR-G tokens to LLM vocabulary IDs
    /// * `vocab_size` - Size of the LLM vocabulary
    pub fn new(mapping: TokenMapping, vocab_size: i32) -> Self {
        let generator = MaskGenerator::new(mapping, vocab_size as usize);
        Self {
            decoder: ConstrainedDecoder::new(generator),
            vocab_size,
        }
    }

    /// Get the vocabulary size
    pub fn vocab_size(&self) -> i32 {
        self.vocab_size
    }

    /// Convert the current mask to llama.cpp logit_bias format
    ///
    /// Returns a vector of `llama_logit_bias` entries that disable
    /// all tokens not allowed in the current parse state.
    ///
    /// # Note
    ///
    /// This allocates a new vector on each call. For high-performance
    /// use cases, consider caching the result or using a more efficient
    /// representation.
    #[cfg(not(target_env = "musl"))]
    pub fn get_logit_bias(&self) -> Vec<arkavo_llama_cpp::ffi::llama_logit_bias> {
        let mask = self.decoder.next_mask();

        (0..self.vocab_size)
            .filter(|&id| !mask.is_allowed(id as u32))
            .map(|id| arkavo_llama_cpp::ffi::llama_logit_bias {
                token: id,
                bias: f32::NEG_INFINITY,
            })
            .collect()
    }

    /// Get the set of allowed token IDs in the current state
    ///
    /// This is useful for debugging or when you need the raw mask
    /// without conversion to logit_bias format.
    pub fn allowed_tokens(&self) -> Vec<u32> {
        let mask = self.decoder.next_mask();
        (0..self.vocab_size as u32)
            .filter(|&id| mask.is_allowed(id))
            .collect()
    }

    /// Feed a generated token back to the decoder
    ///
    /// This advances the internal state machine. Call this after
    /// sampling each token to update the constraints for the next token.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not valid in the current state.
    pub fn feed_token(&mut self, token_id: u32) -> Result<(), TorgError> {
        self.decoder.feed_token(token_id).map_err(TorgError::Decode)
    }

    /// Check if the decoder is in a complete state
    ///
    /// Returns `true` if the current sequence forms a valid TØR-G graph.
    pub fn is_complete(&self) -> bool {
        self.decoder.is_complete()
    }

    /// Finish decoding and extract the graph
    ///
    /// Consumes the sampler and returns the constructed `Graph`.
    ///
    /// # Errors
    ///
    /// Returns an error if the sequence is incomplete or invalid.
    pub fn finish(self) -> Result<Graph, TorgError> {
        self.decoder.finish().map_err(TorgError::Build)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Qwen3TokenMap;

    #[test]
    fn test_sampler_creation() {
        let map = Qwen3TokenMap::mock();
        let sampler = TorgLlamaSampler::new(map.into_mapping(), 151936);

        assert_eq!(sampler.vocab_size(), 151936);
        assert!(!sampler.is_complete());
    }

    #[test]
    fn test_allowed_tokens() {
        let map = Qwen3TokenMap::mock();
        let sampler = TorgLlamaSampler::new(map.into_mapping(), 151936);

        let allowed = sampler.allowed_tokens();
        // Initial state should allow some tokens (InputDecl, NodeStart, OutputDecl, etc.)
        assert!(!allowed.is_empty());
    }
}
