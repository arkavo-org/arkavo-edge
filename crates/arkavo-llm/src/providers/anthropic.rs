use crate::common::{HttpClientBuilder, HttpClientConfig, RetryableHttpClient};
use crate::common::{ProviderError, ProviderResult};
use crate::provider::ProviderResponse;
use crate::tool_parser::ParsedToolCall;
use crate::{Message, Provider, Role, StreamResponse};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::sync::Arc;

/// Anthropic API configuration
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    /// API key for authentication
    pub api_key: String,
    /// Base URL for API
    pub base_url: String,
    /// Model to use (e.g., "claude-3-opus-20240229")
    pub model: String,
    /// API version
    pub api_version: String,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.anthropic.com".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            api_version: "2023-06-01".to_string(),
        }
    }
}

/// Tool definition for Anthropic API
#[derive(Debug, Clone, Serialize)]
struct ToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
}

/// `thinking` request parameter. Adaptive-surface models reject an explicit
/// `{"type": "disabled"}` (Fable 5 returns 400), so the field is omitted
/// entirely when thinking is off rather than sent as disabled.
#[derive(Debug, Clone, Serialize)]
struct ThinkingConfig {
    #[serde(rename = "type")]
    mode: &'static str,
}

impl ThinkingConfig {
    fn adaptive() -> Self {
        Self { mode: "adaptive" }
    }
}

/// Claude models on the adaptive-thinking API surface (Fable 5 and Opus 4.7+).
/// These reject sampling parameters (`temperature`, `top_p`, `top_k`) and
/// fixed thinking budgets with HTTP 400; requests must omit them and opt into
/// `thinking: {"type": "adaptive"}` instead.
fn uses_adaptive_thinking(model: &str) -> bool {
    model.starts_with("claude-fable")
        || model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8")
}

/// Anthropic API request structures
#[derive(Debug, Clone, Serialize)]
struct CreateMessageRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageResponse {
    id: String,
    content: Vec<ResponseContentBlock>,
    model: String,
    stop_reason: Option<String>,
    usage: Usage,
}

/// Content block in response - can be text, tool_use, or thinking
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ResponseContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
}

/// Streaming response structures
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartData },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: ContentBlockStartData,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: DeltaData },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaData,
        usage: Usage,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: ErrorData },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageStartData {
    id: String,
    model: String,
    usage: Usage,
}

/// Content block start data for streaming
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum ContentBlockStartData {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(other)]
    Other,
}

/// Delta data for streaming content blocks
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum DeltaData {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageDeltaData {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ErrorData {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

/// Error response structure
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ErrorResponse {
    #[serde(rename = "type")]
    error_type: String,
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    error_subtype: String,
    message: String,
}

/// Anthropic provider implementation
pub struct AnthropicProvider {
    config: AnthropicConfig,
    client: Arc<RetryableHttpClient>,
}

impl AnthropicProvider {
    /// Create provider from environment variables
    pub fn from_env() -> ProviderResult<Self> {
        Self::from_env_with_model("claude-sonnet-4-5-20250929")
    }

    /// Create provider from environment with an explicit model id.
    ///
    /// `ANTHROPIC_MODEL` still wins when set so operators can pin a model
    /// globally; otherwise the routed model id is sent to the API. Without
    /// this, every routing decision collapses to the env/default model.
    pub fn from_env_with_model(model: &str) -> ProviderResult<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY environment variable not set"))?;

        let model = env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| model.to_string());

        let config = AnthropicConfig {
            api_key,
            model,
            ..Default::default()
        };

        Self::new(config)
    }

    /// Try to create provider from env, returning None if not configured
    pub fn try_from_env() -> Option<Self> {
        Self::from_env().ok()
    }

    pub fn new(config: AnthropicConfig) -> ProviderResult<Self> {
        // Validate base URL
        url::Url::parse(&config.base_url)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;

        let http_config = HttpClientConfig {
            base_url: config.base_url.clone(),
            auth_token: None, // Anthropic uses a custom header
            // Adaptive-thinking models (Fable 5, Opus 4.7+) can reason for tens
            // of seconds before a non-streaming response completes.
            timeout_secs: 120,
            max_retries: 3,
            initial_retry_delay_ms: 1000,
            backoff_factor: 2.0,
            max_retry_delay_ms: 30000,
            jitter_factor: 0.1,
            ..Default::default()
        };

        let builder = HttpClientBuilder::new(http_config);
        let client = Arc::new(RetryableHttpClient::new(builder)?);

        Ok(Self { config, client })
    }

    /// Build a request honoring the configured model's API surface.
    ///
    /// Adaptive-surface models get `thinking: {"type": "adaptive"}` with no
    /// sampling parameters (sending them is a 400), plus a larger default
    /// output ceiling because thinking tokens count toward `max_tokens` and
    /// would truncate answers at the legacy 4096 cap.
    fn build_request(
        &self,
        messages: Vec<ApiMessage>,
        system: Option<String>,
        stream: bool,
        tools: Option<Vec<ToolDefinition>>,
        max_tokens: Option<u32>,
    ) -> CreateMessageRequest {
        let adaptive = uses_adaptive_thinking(&self.config.model);
        let default_max_tokens = if adaptive { 16_000 } else { 4_096 };

        CreateMessageRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: Some(max_tokens.unwrap_or(default_max_tokens)),
            temperature: if adaptive { None } else { Some(0.7) },
            thinking: adaptive.then(ThinkingConfig::adaptive),
            system,
            stream: Some(stream),
            tools,
        }
    }

    /// Convert messages to Anthropic's 3-role format
    fn convert_messages(&self, messages: Vec<Message>) -> (Option<String>, Vec<ApiMessage>) {
        let mut system_content = None;
        let mut api_messages = Vec::new();

        for msg in messages {
            // Skip empty messages (except we'll handle final assistant specially later)
            let content = msg.content.trim();

            match msg.role {
                Role::System => {
                    // Anthropic handles system messages separately
                    if !content.is_empty() {
                        if system_content.is_none() {
                            system_content = Some(content.to_string());
                        } else {
                            // If multiple system messages, concatenate them
                            system_content =
                                Some(format!("{}\n\n{}", system_content.unwrap(), content));
                        }
                    }
                }
                Role::User => {
                    // Skip empty user messages
                    if !content.is_empty() {
                        api_messages.push(ApiMessage {
                            role: "user".to_string(),
                            content: content.to_string(),
                        });
                    }
                }
                Role::Assistant | Role::Tool => {
                    // Skip empty assistant messages (unless it's the last one)
                    if !content.is_empty() {
                        api_messages.push(ApiMessage {
                            role: "assistant".to_string(),
                            content: content.to_string(),
                        });
                    }
                }
            }
        }

        // Ensure conversation starts with user message
        if api_messages.is_empty() || api_messages[0].role != "user" {
            api_messages.insert(
                0,
                ApiMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
            );
        }

        // Ensure alternating user/assistant messages
        let mut cleaned_messages: Vec<ApiMessage> = Vec::new();
        let mut last_role = None;

        for msg in api_messages {
            if last_role.as_ref() == Some(&msg.role) {
                // Same role as previous, merge content
                if let Some(last_msg) = cleaned_messages.last_mut() {
                    last_msg.content = format!("{}\n\n{}", last_msg.content, msg.content);
                }
            } else {
                cleaned_messages.push(msg.clone());
                last_role = Some(msg.role);
            }
        }

        (system_content, cleaned_messages)
    }

    /// Convert tools JSON to Anthropic format
    fn convert_tools_to_definitions(
        tools_json: &Value,
    ) -> Result<Vec<ToolDefinition>, crate::Error> {
        let tools_array = tools_json
            .as_array()
            .ok_or_else(|| crate::Error::Provider("Tools must be an array".into()))?;

        let mut definitions = Vec::new();

        for tool in tools_array {
            // Handle both direct tool format and Gemini-style functionDeclarations wrapper
            if let Some(func_decls) = tool.get("functionDeclarations") {
                // Gemini format: [{functionDeclarations: [...]}]
                let decls = func_decls.as_array().ok_or_else(|| {
                    crate::Error::Provider("functionDeclarations must be array".into())
                })?;
                for decl in decls {
                    definitions.push(Self::parse_tool_definition(decl)?);
                }
            } else {
                // Direct format: [{name, description, input_schema/parameters}]
                definitions.push(Self::parse_tool_definition(tool)?);
            }
        }

        Ok(definitions)
    }

    fn parse_tool_definition(tool: &Value) -> Result<ToolDefinition, crate::Error> {
        let name = tool
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Provider("Tool missing name".into()))?
            .to_string();

        let description = tool
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Provider("Tool missing description".into()))?
            .to_string();

        // Anthropic uses input_schema, but accept parameters as fallback
        let input_schema = tool
            .get("input_schema")
            .or_else(|| tool.get("parameters"))
            .cloned()
            .ok_or_else(|| crate::Error::Provider("Tool missing input_schema/parameters".into()))?;

        Ok(ToolDefinition {
            name,
            description,
            input_schema,
        })
    }

    /// Handle API errors
    async fn handle_error_response(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status();
        let headers = response.headers().clone();

        // Try to parse error body
        if let Ok(error_response) = response.json::<ErrorResponse>().await {
            let error = &error_response.error;

            match error.error_subtype.as_str() {
                "rate_limit_error" => {
                    ProviderError::rate_limited_from_headers(&headers, Some(error.message.clone()))
                }
                "authentication_error" => ProviderError::AuthenticationFailed {
                    message: error.message.clone(),
                    provider: "anthropic".to_string(),
                },
                "not_found_error" => {
                    if error.message.contains("model") {
                        ProviderError::ModelNotFound {
                            model: self.config.model.clone(),
                            provider: "anthropic".to_string(),
                            available_models: None,
                        }
                    } else {
                        ProviderError::InvalidRequest {
                            message: error.message.clone(),
                            details: None,
                        }
                    }
                }
                "invalid_request_error" => ProviderError::InvalidRequest {
                    message: error.message.clone(),
                    details: None,
                },
                _ => ProviderError::InternalError {
                    message: error.message.clone(),
                    provider: "anthropic".to_string(),
                    error_code: Some(error.error_subtype.clone()),
                },
            }
        } else {
            // Fallback error handling
            match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    ProviderError::rate_limited_from_headers(&headers, None)
                }
                StatusCode::UNAUTHORIZED => ProviderError::AuthenticationFailed {
                    message: "Invalid API key".to_string(),
                    provider: "anthropic".to_string(),
                },
                _ => ProviderError::Other(anyhow::anyhow!("Anthropic API error: {status}")),
            }
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String, crate::Error> {
        let (system_content, api_messages) = self.convert_messages(messages);

        let request = self.build_request(api_messages, system_content, false, None, None);

        let url = format!("{}/v1/messages", self.config.base_url);

        let response = self
            .client
            .execute_with_retry(|client| {
                let config = self.config.clone();
                let url = url.clone();
                let request = request.clone();
                Box::pin(async move {
                    let response = client
                        .post(&url)
                        .header("x-api-key", &config.api_key)
                        .header("anthropic-version", &config.api_version)
                        .header("content-type", "application/json")
                        .json(&request)
                        .send()
                        .await?;

                    if response.status().is_success() {
                        let message: MessageResponse = response.json().await?;

                        let content = message
                            .content
                            .iter()
                            .filter_map(|block| match block {
                                ResponseContentBlock::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        Ok(content)
                    } else {
                        // Need to handle error here without self reference
                        let status = response.status();
                        let error_text = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "Failed to read error response".to_string());
                        Err(anyhow::anyhow!(
                            "Anthropic API error {status}: {error_text}"
                        ))
                    }
                })
            })
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        Ok(response)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<
        Box<dyn tokio_stream::Stream<Item = Result<StreamResponse, crate::Error>> + Send + Unpin>,
        crate::Error,
    > {
        let (system_content, api_messages) = self.convert_messages(messages);

        let request = self.build_request(api_messages, system_content, true, None, None);

        let url = format!("{}/v1/messages", self.config.base_url);

        let response = self
            .client
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        if !response.status().is_success() {
            let error = self.handle_error_response(response).await;
            return Err(crate::Error::Provider(error.to_string()));
        }

        // Convert response body to stream of parsed events
        // Use bounded channel to prevent memory exhaustion under load
        let (tx, rx) = tokio::sync::mpsc::channel(1024);

        // Spawn task to process the response stream
        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut stream = response.bytes_stream();

            while let Some(chunk_result) = futures::StreamExt::next(&mut stream).await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        let lines: Vec<String> = buffer
                            .lines()
                            .map(std::string::ToString::to_string)
                            .collect();

                        for line in &lines {
                            if let Some(data) = line.strip_prefix("data: ")
                                && let Ok(event) = serde_json::from_str::<StreamEvent>(data)
                            {
                                let delivery = match event {
                                    StreamEvent::ContentBlockDelta {
                                        delta: DeltaData::TextDelta { text },
                                        ..
                                    } => Some((text, false)),
                                    StreamEvent::MessageStop => Some((String::new(), true)),
                                    StreamEvent::Error { error } => {
                                        let _ = tx
                                            .send(Err(crate::Error::Provider(format!(
                                                "Stream error: {}",
                                                error.message
                                            ))))
                                            .await;
                                        break;
                                    }
                                    StreamEvent::ContentBlockDelta {
                                        delta: DeltaData::InputJsonDelta { .. },
                                        ..
                                    } => None,
                                    _ => None,
                                };
                                if let Some((content, done)) = delivery
                                    && tx
                                        .send(Ok(StreamResponse {
                                            content,
                                            reasoning_content: None,
                                            done,
                                            inference_timing: None,
                                        }))
                                        .await
                                        .is_err()
                                {
                                    break; // Receiver dropped
                                }
                            }
                        }

                        // Clear processed lines from buffer
                        if let Some(last_newline) = buffer.rfind('\n') {
                            buffer.drain(..=last_newline);
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(crate::Error::Provider(e.to_string()))).await;
                        break;
                    }
                }
            }
        });

        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse, crate::Error> {
        let (system_content, api_messages) = self.convert_messages(messages);

        let tool_definitions = tools
            .as_ref()
            .map(Self::convert_tools_to_definitions)
            .transpose()?;

        let request = self.build_request(
            api_messages,
            system_content,
            false,
            tool_definitions,
            max_tokens.map(|t| t as u32),
        );

        let url = format!("{}/v1/messages", self.config.base_url);

        let response = self
            .client
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        if !response.status().is_success() {
            let error = self.handle_error_response(response).await;
            return Err(crate::Error::Provider(error.to_string()));
        }

        let message: MessageResponse = response
            .json()
            .await
            .map_err(|e| crate::Error::Provider(format!("Failed to parse response: {e}")))?;

        // Extract text content and tool calls from response
        let mut text_content = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();

        for block in message.content {
            match block {
                ResponseContentBlock::Text { text } => {
                    if !text_content.is_empty() {
                        text_content.push('\n');
                    }
                    text_content.push_str(&text);
                }
                ResponseContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ParsedToolCall {
                        tool_name: name,
                        arguments: input,
                        call_id: Some(id),
                    });
                }
                ResponseContentBlock::Thinking { thinking } => {
                    reasoning.push_str(&thinking);
                }
                ResponseContentBlock::Other => {}
            }
        }

        Ok(ProviderResponse {
            content: text_content,
            reasoning_content: if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
            tool_calls,
            finish_reason: message.stop_reason,
            inference_timing: None,
            quality_gate_retries: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversion() {
        let config = AnthropicConfig::default();
        let provider = AnthropicProvider::new(config).unwrap();

        let messages = vec![
            Message::system("You are a helpful assistant"),
            Message::user("Hello"),
            Message::assistant("Hi there!"),
            Message::user("How are you?"),
        ];

        let (system, api_messages) = provider.convert_messages(messages);

        assert_eq!(system, Some("You are a helpful assistant".to_string()));
        assert_eq!(api_messages.len(), 3);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[1].role, "assistant");
        assert_eq!(api_messages[2].role, "user");
    }

    #[test]
    fn test_message_deduplication() {
        let config = AnthropicConfig::default();
        let provider = AnthropicProvider::new(config).unwrap();

        let messages = vec![
            Message::user("First message"),
            Message::user("Second message"),
            Message::assistant("Response"),
        ];

        let (_, api_messages) = provider.convert_messages(messages);

        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].content, "First message\n\nSecond message");
        assert_eq!(api_messages[1].role, "assistant");
    }

    #[test]
    fn test_tool_conversion_direct_format() {
        use serde_json::json;

        let tools = json!([
            {
                "name": "get_weather",
                "description": "Get weather for a location",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }
            }
        ]);

        let definitions = AnthropicProvider::convert_tools_to_definitions(&tools).unwrap();

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "get_weather");
        assert_eq!(definitions[0].description, "Get weather for a location");
    }

    #[test]
    fn test_tool_conversion_gemini_format() {
        use serde_json::json;

        // Gemini format with functionDeclarations wrapper
        let tools = json!([
            {
                "functionDeclarations": [
                    {
                        "name": "search_files",
                        "description": "Search for files",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "query": {"type": "string"}
                            }
                        }
                    }
                ]
            }
        ]);

        let definitions = AnthropicProvider::convert_tools_to_definitions(&tools).unwrap();

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "search_files");
    }

    #[test]
    fn test_default_model_is_sonnet() {
        let config = AnthropicConfig::default();
        assert_eq!(config.model, "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn test_supports_tools() {
        let config = AnthropicConfig::default();
        let provider = AnthropicProvider::new(config).unwrap();
        assert!(provider.supports_tools());
    }

    #[test]
    fn test_adaptive_thinking_surface_detection() {
        assert!(uses_adaptive_thinking("claude-fable-5"));
        assert!(uses_adaptive_thinking("claude-opus-4-7"));
        assert!(uses_adaptive_thinking("claude-opus-4-8"));
        assert!(!uses_adaptive_thinking("claude-sonnet-4-5-20250929"));
        assert!(!uses_adaptive_thinking("claude-opus-4-5-20251101"));
        assert!(!uses_adaptive_thinking("kimi-k2.5"));
    }

    // Regression: Fable 5 / Opus 4.7+ return HTTP 400 if `temperature` or an
    // explicit `thinking: {"type": "disabled"}` is sent. Requests for these
    // models must omit sampling params and opt into adaptive thinking.
    #[test]
    fn test_fable_request_omits_sampling_and_uses_adaptive_thinking() {
        let config = AnthropicConfig {
            model: "claude-fable-5".to_string(),
            ..Default::default()
        };
        let provider = AnthropicProvider::new(config).unwrap();

        let request = provider.build_request(
            vec![ApiMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            None,
            false,
            None,
            None,
        );

        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("temperature").is_none(), "must omit temperature");
        assert_eq!(json["thinking"]["type"], "adaptive");
        // Thinking tokens count toward max_tokens; 4096 truncates answers.
        assert_eq!(json["max_tokens"], 16_000);
    }

    #[test]
    fn test_legacy_model_request_keeps_sampling_without_thinking() {
        let config = AnthropicConfig::default(); // Sonnet 4.5
        let provider = AnthropicProvider::new(config).unwrap();

        let request = provider.build_request(
            vec![ApiMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            None,
            false,
            None,
            None,
        );

        let json = serde_json::to_value(&request).unwrap();
        let temperature = json["temperature"].as_f64().expect("temperature present");
        assert!((temperature - 0.7).abs() < 1e-6);
        assert!(json.get("thinking").is_none(), "must omit thinking field");
        assert_eq!(json["max_tokens"], 4096);
    }

    #[test]
    fn test_explicit_max_tokens_overrides_default() {
        let config = AnthropicConfig {
            model: "claude-fable-5".to_string(),
            ..Default::default()
        };
        let provider = AnthropicProvider::new(config).unwrap();

        let request = provider.build_request(Vec::new(), None, false, None, Some(2048));
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["max_tokens"], 2048);
    }

    #[test]
    fn test_thinking_content_block_deserialization() {
        use serde_json::json;

        // Simulate response with thinking + text content blocks (Kimi K2.5 via Anthropic API)
        let response_json = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "kimi-k2.5",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "Let me reason about 2+2. It equals 4."
                },
                {
                    "type": "text",
                    "text": "4"
                }
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20
            }
        });

        let message: MessageResponse = serde_json::from_value(response_json).unwrap();
        assert_eq!(message.content.len(), 2);

        // Extract text (should skip thinking blocks)
        let text: String = message
            .content
            .iter()
            .filter_map(|block| match block {
                ResponseContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(text, "4");

        // Extract reasoning
        let reasoning: String = message
            .content
            .iter()
            .filter_map(|block| match block {
                ResponseContentBlock::Thinking { thinking } => Some(thinking.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(reasoning, "Let me reason about 2+2. It equals 4.");
    }

    #[test]
    fn test_streaming_thinking_delta_deserialization() {
        // Thinking delta events should deserialize without error
        let thinking_delta_json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Step 1..."}}"#;
        let event: StreamEvent = serde_json::from_str(thinking_delta_json).unwrap();
        assert!(matches!(
            event,
            StreamEvent::ContentBlockDelta {
                delta: DeltaData::ThinkingDelta { .. },
                ..
            }
        ));

        // Text delta still works
        let text_delta_json = r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let event: StreamEvent = serde_json::from_str(text_delta_json).unwrap();
        assert!(matches!(
            event,
            StreamEvent::ContentBlockDelta {
                delta: DeltaData::TextDelta { .. },
                ..
            }
        ));

        // Unknown delta types are handled gracefully
        let unknown_delta_json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"future_delta","data":"..."}}"#;
        let event: StreamEvent = serde_json::from_str(unknown_delta_json).unwrap();
        assert!(matches!(
            event,
            StreamEvent::ContentBlockDelta {
                delta: DeltaData::Other,
                ..
            }
        ));
    }
}
