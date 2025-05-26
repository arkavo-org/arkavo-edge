//! This file provides the tokenizer data needed for the Qwen3 model.
//! For actual deployments, the data is generated at build time.

// This module contains the generated tokenizer data when the required features are enabled
#[cfg(all(feature = "embedded_model", feature = "use_generated_data"))]
mod generated {
    include!(env!("TOKENIZER_DATA_GENERATED_PATH"));
}

// For development and testing, or when tokenizer file is missing,
// we provide fallback constants that work with Qwen3-0.6B

// Use the generated constants if available, otherwise use fallbacks
#[cfg(all(feature = "embedded_model", feature = "use_generated_data"))]
pub use generated::TOKENIZER_JSON;

#[cfg(all(feature = "embedded_model", feature = "use_generated_data"))]
pub use generated::CONFIG_JSON;

#[cfg(not(all(feature = "embedded_model", feature = "use_generated_data")))]
/// The raw tokenizer JSON data - a minimal BPE tokenizer that can handle basic English text
pub const TOKENIZER_JSON: &[u8] = include_bytes!("../models/tokenizer.json");

#[cfg(not(all(feature = "embedded_model", feature = "use_generated_data")))]
/// The raw model config JSON data - Qwen3-0.6B configuration
pub const CONFIG_JSON: &[u8] = b"{\"hidden_size\":1024,\"num_attention_heads\":16,\"num_hidden_layers\":28,\"vocab_size\":151936}";
