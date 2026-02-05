//! Native Kimi (Moonshot) API client for Rust.
//!
//! Provides direct access to Kimi API features without OpenAI abstraction layers.
//!
//! # Kimi K2.5 Series Support
//!
//! This crate supports the latest Kimi K2.5 series models:
//! - `kimi-k2.5` - General purpose model with 256K context
//! - `kimi-k2-0905-preview` - Preview version
//! - `kimi-k2-turbo-preview` - Faster responses
//! - `kimi-k2-thinking` - Enhanced reasoning capabilities
//! - `kimi-k2-thinking-turbo` - Fast reasoning
//!
//! All K2.5 models support:
//! - 256K context window
//! - Thinking mode (can be enabled/disabled)
//! - Multimodal inputs (vision)
//! - Tool calling

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
    ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChatRole, Model, ThinkingConfig,
    ThinkingMode, Tool, ToolChoice, ToolFunction,
};
