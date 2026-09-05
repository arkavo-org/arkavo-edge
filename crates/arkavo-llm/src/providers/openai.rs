use crate::common::{HttpClientBuilder, HttpClientConfig, RetryableHttpClient};
use crate::common::{ProviderError, ProviderResult};
use crate::provider::{InferenceTiming, ProviderResponse};
use crate::tool_parser::ParsedToolCall;
use crate::{Message, Provider, Role, StreamResponse};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// OpenAI API configuration
#[derive(Clone, Debug)]
pub struct OpenAIConfig {
    /// API key for authentication
    pub api_key: String,
    /// Base URL (for OpenAI or Azure endpoints)
    pub base_url: String,
    /// Model to use
    pub model: String,
    /// Organization ID (optional)
    pub organization_id: Option<String>,
    /// API version (for Azure)
    pub api_version: Option<String>,
    /// Whether this is an Azure endpoint
    pub is_azure: bool,
}

impl Default for OpenAIConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4".to_string(),
            organization_id: None,
            api_version: None,
            is_azure: false,
        }
    }
}

/// OpenAI API request structures
#[derive(Debug, Clone, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    /// OpenAI function-calling tool list. Omitted entirely when the caller
    /// passes no tools, so plain chat requests are unchanged on the wire.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    /// Null for an assistant turn that is purely tool calls; otherwise the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// Tool calls the assistant issued (response) or is replaying (request).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallJson>>,
    /// Set on `role:"tool"` result messages, pairing the result to its call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// OpenAI tool-call wire shape: `{id, type:"function", function:{name, arguments}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type", default = "function_call_type")]
    call_type: String,
    function: FunctionCallJson,
}

fn function_call_type() -> String {
    "function".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionCallJson {
    name: String,
    /// Arguments as a JSON string (OpenAI sends them stringified).
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Choice {
    message: ApiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[allow(clippy::struct_field_names)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

/// Streaming response structures
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

/// Error response structure
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ErrorDetail {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
    code: Option<String>,
}

/// OpenAI provider implementation
pub struct OpenAIProvider {
    config: OpenAIConfig,
    client: Arc<RetryableHttpClient>,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIConfig) -> ProviderResult<Self> {
        // Validate base URL
        url::Url::parse(&config.base_url)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;

        let http_config = HttpClientConfig {
            base_url: config.base_url.clone(),
            auth_token: Some(config.api_key.clone()),
            timeout_secs: 60,
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

    /// Convert internal messages to API format, including assistant tool calls
    /// and `role:"tool"` results so a multi-turn tool loop replays correctly.
    fn convert_messages(&self, messages: Vec<Message>) -> Vec<ApiMessage> {
        messages
            .into_iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                }
                .to_string();

                let tool_calls = (!msg.tool_calls.is_empty()).then(|| {
                    msg.tool_calls
                        .iter()
                        .map(|tc| ToolCallJson {
                            id: tc.id.clone(),
                            call_type: "function".to_string(),
                            function: FunctionCallJson {
                                name: tc.name.clone(),
                                arguments: tc.arguments.clone(),
                            },
                        })
                        .collect()
                });

                // OpenAI requires `content: null` (not "") on an assistant turn
                // that only carries tool calls.
                let content = if msg.content.is_empty() && tool_calls.is_some() {
                    None
                } else {
                    Some(msg.content)
                };

                ApiMessage {
                    role,
                    content,
                    tool_calls,
                    tool_call_id: msg.tool_call_id,
                }
            })
            .collect()
    }

    /// Convert the router's generic tool list (`[{name, description,
    /// parameters|input_schema}]`) into OpenAI's `tools` array.
    fn convert_tools_to_openai(tools_json: &Value) -> Vec<Value> {
        let Some(arr) = tools_json.as_array() else {
            return Vec::new();
        };
        arr.iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?;
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let parameters = tool
                    .get("parameters")
                    .or_else(|| tool.get("input_schema"))
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                Some(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters,
                    }
                }))
            })
            .collect()
    }

    /// Parse a response message's `tool_calls` into the unified representation.
    fn parse_tool_calls(message: &ApiMessage) -> Vec<ParsedToolCall> {
        message
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .map(|tc| ParsedToolCall {
                        tool_name: tc.function.name.clone(),
                        // Arguments arrive as a JSON string; parse to a value,
                        // falling back to an empty object on malformed JSON.
                        arguments: serde_json::from_str(&tc.function.arguments)
                            .unwrap_or_else(|_| json!({})),
                        call_id: tc.id.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Map the response `usage` block (prompt/completion tokens) onto
    /// `InferenceTiming` so the cost path can reconcile against real spend.
    /// Cloud usage carries no wall-clock timing, only token counts.
    fn timing_from_usage(usage: &Usage) -> InferenceTiming {
        InferenceTiming {
            n_prompt_eval: usage.prompt_tokens,
            n_eval: usage.completion_tokens,
            ..Default::default()
        }
    }

    /// Build the API endpoint URL
    fn build_url(&self, endpoint: &str) -> String {
        if self.config.is_azure {
            // Azure OpenAI endpoint format
            format!(
                "{}/openai/deployments/{}/{}?api-version={}",
                self.config.base_url,
                self.config.model,
                endpoint,
                self.config.api_version.as_deref().unwrap_or("2024-02-01")
            )
        } else {
            // Standard OpenAI endpoint
            format!("{}/{}", self.config.base_url, endpoint)
        }
    }

    /// Handle API errors
    async fn handle_error_response(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status();
        let headers = response.headers().clone();

        // Try to parse error body
        if let Ok(error_response) = response.json::<ErrorResponse>().await {
            let error = &error_response.error;

            match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    ProviderError::rate_limited_from_headers(&headers, Some(error.message.clone()))
                }
                StatusCode::UNAUTHORIZED => ProviderError::AuthenticationFailed {
                    message: error.message.clone(),
                    provider: "openai".to_string(),
                },
                StatusCode::NOT_FOUND => {
                    if error.message.contains("model") {
                        ProviderError::ModelNotFound {
                            model: self.config.model.clone(),
                            provider: "openai".to_string(),
                            available_models: None,
                        }
                    } else {
                        ProviderError::InvalidRequest {
                            message: error.message.clone(),
                            details: None,
                        }
                    }
                }
                StatusCode::BAD_REQUEST => ProviderError::InvalidRequest {
                    message: error.message.clone(),
                    details: None,
                },
                _ if status.is_server_error() => ProviderError::InternalError {
                    message: error.message.clone(),
                    provider: "openai".to_string(),
                    error_code: error.code.clone(),
                },
                _ => ProviderError::Other(anyhow::anyhow!("OpenAI API error: {}", error.message)),
            }
        } else {
            // Fallback error handling
            match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    ProviderError::rate_limited_from_headers(&headers, None)
                }
                StatusCode::UNAUTHORIZED => ProviderError::AuthenticationFailed {
                    message: "Invalid API key".to_string(),
                    provider: "openai".to_string(),
                },
                _ => ProviderError::Other(anyhow::anyhow!("OpenAI API error: {status}")),
            }
        }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String, crate::Error> {
        let api_messages = self.convert_messages(messages);

        // GPT-5 only supports default temperature (1.0)
        let temperature = if self.config.model == "gpt-5" {
            None // Use default
        } else {
            Some(0.7)
        };

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            temperature,
            max_tokens: None,
            stream: Some(false),
            n: Some(1),
            tools: None,
            tool_choice: None,
        };

        let url = self.build_url("chat/completions");

        let response = self
            .client
            .execute_with_retry(|client| {
                let config = self.config.clone();
                let url = url.clone();
                let request = request.clone();
                Box::pin(async move {
                    let mut req = client.post(&url).json(&request);

                    // Add Azure-specific header if needed
                    if config.is_azure {
                        req = req.header("api-key", &config.api_key);
                    }

                    // Add organization header if provided
                    if let Some(ref org_id) = config.organization_id {
                        req = req.header("OpenAI-Organization", org_id);
                    }

                    let response = req.send().await?;

                    if response.status().is_success() {
                        let completion: ChatCompletionResponse = response.json().await?;

                        completion
                            .choices
                            .first()
                            .map(|choice| choice.message.content.clone().unwrap_or_default())
                            .ok_or_else(|| anyhow::anyhow!("No response from OpenAI"))
                    } else {
                        // Need to handle error here without self reference
                        let status = response.status();
                        let error_text = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "Failed to read error response".to_string());
                        Err(anyhow::anyhow!("OpenAI API error {status}: {error_text}"))
                    }
                })
            })
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        Ok(response)
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
        let api_messages = self.convert_messages(messages);

        let temperature = if self.config.model == "gpt-5" {
            None
        } else {
            Some(0.7)
        };

        // Attach tools only when the caller passed a non-empty set; otherwise
        // the field stays off the wire and this is an ordinary completion.
        let openai_tools = tools
            .as_ref()
            .map(Self::convert_tools_to_openai)
            .filter(|t| !t.is_empty());
        let tool_choice = openai_tools.as_ref().map(|_| json!("auto"));

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            temperature,
            max_tokens: max_tokens.and_then(|m| u32::try_from(m).ok()),
            stream: Some(false),
            n: Some(1),
            tools: openai_tools,
            tool_choice,
        };

        let url = self.build_url("chat/completions");

        let (content, tool_calls, finish_reason, inference_timing) = self
            .client
            .execute_with_retry(|client| {
                let config = self.config.clone();
                let url = url.clone();
                let request = request.clone();
                Box::pin(async move {
                    let mut req = client.post(&url).json(&request);
                    if config.is_azure {
                        req = req.header("api-key", &config.api_key);
                    }
                    if let Some(ref org_id) = config.organization_id {
                        req = req.header("OpenAI-Organization", org_id);
                    }

                    let response = req.send().await?;
                    if response.status().is_success() {
                        let completion: ChatCompletionResponse = response.json().await?;
                        let choice = completion
                            .choices
                            .first()
                            .ok_or_else(|| anyhow::anyhow!("No response from OpenAI"))?;
                        let content = choice.message.content.clone().unwrap_or_default();
                        let tool_calls = OpenAIProvider::parse_tool_calls(&choice.message);
                        let finish_reason = choice.finish_reason.clone();
                        let timing = completion
                            .usage
                            .as_ref()
                            .map(OpenAIProvider::timing_from_usage);
                        Ok((content, tool_calls, finish_reason, timing))
                    } else {
                        let status = response.status();
                        let error_text = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "Failed to read error response".to_string());
                        Err(anyhow::anyhow!("OpenAI API error {status}: {error_text}"))
                    }
                })
            })
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        Ok(ProviderResponse {
            response_items: Vec::new(),
            content,
            reasoning_content: None,
            tool_calls,
            finish_reason,
            inference_timing,
            quality_gate_retries: 0,
        })
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<
        Box<dyn tokio_stream::Stream<Item = Result<StreamResponse, crate::Error>> + Send + Unpin>,
        crate::Error,
    > {
        let api_messages = self.convert_messages(messages);

        // GPT-5 only supports default temperature (1.0)
        let temperature = if self.config.model == "gpt-5" {
            None // Use default
        } else {
            Some(0.7)
        };

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            temperature,
            max_tokens: None,
            stream: Some(true),
            n: Some(1),
            tools: None,
            tool_choice: None,
        };

        let url = self.build_url("chat/completions");
        let api_key = self.config.api_key.clone();
        let is_azure = self.config.is_azure;
        let org_id = self.config.organization_id.clone();

        let mut req = self.client.client.post(&url).json(&request);

        if is_azure {
            req = req.header("api-key", &api_key);
        } else {
            req = req.header("Authorization", format!("Bearer {api_key}"));
        }

        if let Some(ref org) = org_id {
            req = req.header("OpenAI-Organization", org);
        }

        let response = req
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
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    if tx
                                        .send(Ok(StreamResponse {
                                            response_items: Vec::new(),
                                            content: String::new(),
                                            reasoning_content: None,
                                            done: true,
                                            inference_timing: None,
                                        }))
                                        .await
                                        .is_err()
                                    {
                                        break; // Receiver dropped
                                    }
                                } else if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data)
                                    && let Some(choice) = chunk.choices.first()
                                    && let Some(content) = &choice.delta.content
                                    && tx
                                        .send(Ok(StreamResponse {
                                            response_items: Vec::new(),
                                            content: content.clone(),
                                            reasoning_content: None,
                                            done: choice.finish_reason.is_some(),
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
        "openai"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_azure_url_building() {
        let config = OpenAIConfig {
            base_url: "https://myinstance.openai.azure.com".to_string(),
            model: "gpt-4".to_string(),
            is_azure: true,
            api_version: Some("2024-02-01".to_string()),
            ..Default::default()
        };

        let provider = OpenAIProvider::new(config).unwrap();
        let url = provider.build_url("chat/completions");

        assert_eq!(
            url,
            "https://myinstance.openai.azure.com/openai/deployments/gpt-4/chat/completions?api-version=2024-02-01"
        );
    }

    #[test]
    fn test_openai_url_building() {
        let config = OpenAIConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4".to_string(),
            is_azure: false,
            ..Default::default()
        };

        let provider = OpenAIProvider::new(config).unwrap();
        let url = provider.build_url("chat/completions");

        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_message_conversion() {
        let config = OpenAIConfig::default();
        let provider = OpenAIProvider::new(config).unwrap();

        let messages = vec![
            Message::system("You are a helpful assistant"),
            Message::user("Hello"),
        ];

        let api_messages = provider.convert_messages(messages);

        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].role, "system");
        assert_eq!(api_messages[1].role, "user");
    }

    #[test]
    fn provider_advertises_tool_support() {
        let provider = OpenAIProvider::new(OpenAIConfig::default()).unwrap();
        assert!(
            provider.supports_tools(),
            "OpenAI-compatible providers (incl. GLM) must advertise tool support"
        );
    }

    #[test]
    fn convert_tools_wraps_in_openai_function_shape() {
        let tools = json!([{
            "name": "get_time",
            "description": "Get the current time",
            "parameters": {"type": "object", "properties": {}}
        }]);
        let out = OpenAIProvider::convert_tools_to_openai(&tools);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "function");
        assert_eq!(out[0]["function"]["name"], "get_time");
        assert_eq!(out[0]["function"]["description"], "Get the current time");
        assert!(out[0]["function"]["parameters"].is_object());
    }

    #[test]
    fn convert_tools_accepts_input_schema_alias() {
        // Anthropic-style `input_schema` maps onto OpenAI's `parameters`.
        let tools = json!([{"name": "t", "input_schema": {"type": "object"}}]);
        let out = OpenAIProvider::convert_tools_to_openai(&tools);
        assert_eq!(out[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn parse_tool_calls_extracts_name_args_and_id() {
        let msg = ApiMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCallJson {
                id: Some("call_1".to_string()),
                call_type: "function".to_string(),
                function: FunctionCallJson {
                    name: "get_time".to_string(),
                    arguments: r#"{"tz":"UTC"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
        };
        let parsed = OpenAIProvider::parse_tool_calls(&msg);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].tool_name, "get_time");
        assert_eq!(parsed[0].call_id.as_deref(), Some("call_1"));
        assert_eq!(parsed[0].arguments["tz"], "UTC");
    }

    #[test]
    fn parse_tool_calls_empty_for_plain_text() {
        let msg = ApiMessage {
            role: "assistant".to_string(),
            content: Some("hi".to_string()),
            tool_calls: None,
            tool_call_id: None,
        };
        assert!(OpenAIProvider::parse_tool_calls(&msg).is_empty());
    }

    #[test]
    fn convert_messages_round_trips_call_and_result() {
        let provider = OpenAIProvider::new(OpenAIConfig::default()).unwrap();
        let messages = vec![
            Message {
                response_items: Vec::new(),
                role: Role::Assistant,
                tool_calls: vec![crate::ToolCall {
                    name: "get_time".to_string(),
                    arguments: "{}".to_string(),
                    id: Some("call_1".to_string()),
                }],
                ..Default::default()
            },
            Message {
                response_items: Vec::new(),
                role: Role::Tool,
                content: "12:00 UTC".to_string(),
                tool_call_id: Some("call_1".to_string()),
                tool_name: Some("get_time".to_string()),
                ..Default::default()
            },
        ];
        let api = provider.convert_messages(messages);
        // Assistant turn: null content + tool_calls (OpenAI requires null, not "").
        assert_eq!(api[0].role, "assistant");
        assert!(api[0].content.is_none());
        assert_eq!(
            api[0].tool_calls.as_ref().unwrap()[0].function.name,
            "get_time"
        );
        // Tool result: role=tool + paired tool_call_id + content.
        assert_eq!(api[1].role, "tool");
        assert_eq!(api[1].content.as_deref(), Some("12:00 UTC"));
        assert_eq!(api[1].tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn usage_maps_to_inference_timing() {
        let usage = Usage {
            prompt_tokens: 120,
            completion_tokens: 45,
            total_tokens: 165,
        };
        let timing = OpenAIProvider::timing_from_usage(&usage);
        assert_eq!(timing.n_prompt_eval, 120);
        assert_eq!(timing.n_eval, 45);
    }
}
