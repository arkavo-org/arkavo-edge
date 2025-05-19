//! This file provides direct access to the embedded GGUF model
//! The model data is included directly to ensure it's always available

/// The raw GGUF model data
/// This uses a static include_bytes! macro to embed the model data
/// at compile time, ensuring it's always available
pub const EMBEDDED_MODEL: &[u8] = include_bytes!("../models/Qwen3-0.6B-Q4_K_M.gguf");