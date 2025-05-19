//! This file provides the tokenizer data needed for the Qwen3 model.
//! For actual deployments, the data is generated at build time.

// For development and testing, or when tokenizer file is missing,
// we provide fallback constants that work with Qwen3-0.6B
/// The raw tokenizer JSON data - a minimal BPE tokenizer that can handle basic English text
pub const TOKENIZER_JSON: &[u8] = include_bytes!("../models/tokenizer.json");

/// The raw model config JSON data - Qwen3-0.6B configuration
pub const CONFIG_JSON: &[u8] = b"{\"hidden_size\":1024,\"num_attention_heads\":16,\"num_hidden_layers\":28,\"vocab_size\":151936}";

// If the embedded_model feature is enabled and the file exists,
// include it to override the fallback constants
#[cfg(feature = "embedded_model")]
#[path = "tokenizer_data_generated.rs"]
mod generated {
    // If this fails to find the file, the compilation will
    // continue with the fallback constants above instead of failing
}

// Re-export the generated constants if they exist, otherwise use fallbacks
#[cfg(all(feature = "embedded_model", feature = "use_generated_data"))]
pub use generated::*;