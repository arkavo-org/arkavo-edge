pub mod chat;
pub mod client;
#[cfg(feature = "llm-remote")]
pub mod common;
pub mod config;
pub mod error;
pub mod image;
pub mod mcp_converter;
pub mod message;
#[cfg(feature = "llm-remote")]
pub mod ollama;
pub mod provider;
#[cfg(feature = "llm-remote")]
pub mod providers;
pub mod stream;
pub mod stream_adapter;
pub mod stream_model;
pub mod tool_executor;
pub mod tool_parser;

pub use chat::ChatRequest;
pub use client::LlmClient;
pub use config::LlmConfig;
pub use error::{Error, Result};
pub use image::{ImageFormat, decode_image, encode_image_bytes, encode_image_file};
pub use mcp_converter::{LocalToolFormat, McpConverter};
pub use message::{Message, Role};
pub use provider::{Provider, ProviderResponse};
pub use stream::StreamResponse;
pub use tool_executor::{ToolExecutionError, ToolExecutionResult, ToolExecutor};
pub use tool_parser::{ParsedToolCall, ToolParseError, ToolParser};

#[cfg(feature = "kimi")]
mod kimi_adapter;
#[cfg(feature = "kimi")]
pub use kimi_adapter::KimiProvider;

#[cfg(feature = "deepseek")]
mod deepseek_adapter;
#[cfg(feature = "deepseek")]
pub use deepseek_adapter::DeepSeekProvider;

#[cfg(feature = "qwen")]
mod qwen_adapter;
#[cfg(feature = "qwen")]
pub use qwen_adapter::QwenProvider;

#[cfg(feature = "gemini")]
mod gemini_adapter;
#[cfg(feature = "gemini")]
pub use gemini_adapter::GeminiProvider;

#[cfg(feature = "llama-cpp")]
mod llamacpp_provider;
#[cfg(feature = "llama-cpp")]
mod llamacpp_streaming;
#[cfg(feature = "llama-cpp")]
pub use llamacpp_provider::{LlamaCppProvider, SamplingConfig, is_gpu_accelerated};
pub use stream_adapter::LlmClientAdapter;
pub use stream_model::{
    DeltaStream, DeltaType, EndReason, StreamControl, StreamDelta, StreamError, StreamId,
    StreamLlmModel, create_bounded_stream, text_to_delta_stream,
};
