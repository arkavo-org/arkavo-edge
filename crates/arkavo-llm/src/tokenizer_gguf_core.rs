use std::collections::HashMap;

/// Core structure for the GGUF tokenizer
pub struct GgufTokenizer {
    pub(crate) vocab: HashMap<String, u32>,
    pub(crate) reverse_vocab: HashMap<u32, String>,
    // Instead of Vec, use a merge map for O(1) lookup:
    pub(crate) merges: HashMap<(String, String), String>,
    pub(crate) special_tokens: HashMap<String, u32>,
    #[allow(dead_code)] // Currently unused but might be useful in future optimizations
    pub(crate) max_token_length: usize,
}

impl GgufTokenizer {
    /// Returns the size of the vocabulary
    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }
}