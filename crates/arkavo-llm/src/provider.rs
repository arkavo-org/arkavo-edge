use async_trait::async_trait;
use serde_json::Value;
use tokio_stream::Stream;

use crate::tool_parser::ParsedToolCall;
use crate::{Message, Result, StreamResponse};

/// Response from a provider that may include tool calls
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub content: String,
    pub tool_calls: Vec<ParsedToolCall>,
    pub finish_reason: Option<String>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        self.complete_with_options(messages, None).await
    }

    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String>;

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>>;

    fn name(&self) -> &str;

    /// Check if this provider supports native tool calling
    fn supports_tools(&self) -> bool {
        false
    }

    /// Complete with tool support (returns structured response with tool calls)
    /// Default implementation returns response without tool calls for backward compatibility
    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        _tools: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let content = self.complete_with_options(messages, max_tokens).await?;
        Ok(ProviderResponse {
            content,
            tool_calls: Vec::new(),
            finish_reason: None,
        })
    }
}
