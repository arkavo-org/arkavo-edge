//! Response processing utilities
//!
//! Functions for cleaning and sanitizing LLM responses.

pub mod processing;

pub use processing::{sanitize_response, strip_think_blocks, strip_tool_blocks};
