pub mod anthropic_compat;
pub mod client;
pub mod error;
pub mod provider;
pub mod stream;
pub mod strict_mode;
pub mod types;

// Re-export main types
pub use client::{DeepSeekClient, DeepSeekConfig};
pub use error::{DeepSeekError, Result};
pub use provider::{DeepSeekProvider, Message, MessageRole, Provider};
pub use stream::StreamResponse;
pub use types::{
    AnthropicMessage, AnthropicMessageRequest, AnthropicTool, AnthropicToolChoice,
    ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ContentPart, FunctionCall,
    MessageContent, Model, ResponseFormat, Role, Tool, ToolCall, ToolChoice, ToolFunction,
    V32_SPECIALE_BASE_URL, V32_SPECIALE_EXPIRATION,
};
