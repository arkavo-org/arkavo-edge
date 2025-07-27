//! Native Kimi (Moonshot) API client for Rust.
//!
//! Provides direct access to Kimi API features without OpenAI abstraction layers.

pub mod client;
pub mod error;
pub mod provider;
pub mod retry;
pub mod stream;
pub mod types;

/// Native Kimi API client
pub use client::{KimiClient, KimiConfig};
/// Error types for Kimi API operations
pub use error::{KimiError, Result};
/// Provider trait implementation for arkavo-llm integration
pub use provider::{KimiProvider, Message, Provider, Role, StreamResponse};
/// Request/response types and models
pub use types::{
    ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChatRole, Model, Tool, ToolChoice,
    ToolFunction,
};
