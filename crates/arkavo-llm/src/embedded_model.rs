//! This file provides a placeholder for the embedded GGUF model
//! For real deployments, this should be replaced with actual embedded data

/// Empty placeholder for when no model is embedded.
/// This will be overridden by build.rs if a model file is found.
/// 
/// In a real deployment with an embedded model, the constant would be filled
/// with the actual GGUF model data, which can be several hundred MB or more.
/// 
/// For testing and development, the empty array allows the code to compile
/// even without an actual model file.
pub const EMBEDDED_MODEL: &[u8] = &[];