//! This file provides the tokenizer data needed for the Qwen3 model.
//! For actual deployments, the data is generated at build time.

// Try to include the generated file from build.rs if available
#[cfg(feature = "use_generated_tokenizer")]
include!(concat!(env!("OUT_DIR"), "/tokenizer_data.rs"));

// For development and testing, provide fallback constants
#[cfg(not(feature = "use_generated_tokenizer"))]
/// The raw tokenizer JSON data (minimal placeholder)
pub const TOKENIZER_JSON: &[u8] = b"{\"model\":{\"vocab\":{},\"merges\":[]}}";

#[cfg(not(feature = "use_generated_tokenizer"))]
/// The raw model config JSON data (minimal placeholder)
pub const CONFIG_JSON: &[u8] = b"{\"hidden_size\":1024,\"num_attention_heads\":16,\"num_hidden_layers\":28,\"vocab_size\":151936}";