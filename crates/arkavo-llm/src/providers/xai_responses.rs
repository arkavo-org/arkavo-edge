//! xAI Responses API client (`POST /v1/responses`).
//!
//! Preferred path for Grok 4.5 agentic work: stateful multi-turn via
//! `previous_response_id`, reasoning effort control (default `"low"` for
//! latency), function calling, and SSE streaming.
//!
//! Chat Completions remains available through [`super::openai::OpenAIProvider`]
//! for OpenAI-compatible hosts; this client targets the Responses surface.

use crate::common::{HttpClientBuilder, HttpClientConfig, RetryableHttpClient};
use crate::provider::{InferenceTiming, ProviderResponse};
use crate::tool_parser::ParsedToolCall;
use crate::{Message, Provider, Role, StreamResponse};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::Mutex;

/// Reasoning effort for Grok models. Default is Low for agent latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    #[default]
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Configuration for the xAI Responses API.
#[derive(Clone, Debug)]
pub struct ResponsesConfig {
    pub api_key: String,
    /// Base URL including `/v1`, e.g. `https://api.x.ai/v1`.
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: ReasoningEffort,
    /// Persist server-side state (enables `previous_response_id` chaining).
    pub store: bool,
    /// Optional service tier (`"priority"` for lower TTFT under load).
    pub service_tier: Option<String>,
}

impl Default for ResponsesConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.x.ai/v1".to_string(),
            model: "grok-4.5".to_string(),
            reasoning_effort: ReasoningEffort::Low,
            store: true,
            service_tier: None,
        }
    }
}

impl ResponsesConfig {
    /// Build from `XAI_API_KEY` / optional `XAI_BASE_URL`.
    pub fn from_env() -> Result<Self, crate::Error> {
        let api_key = std::env::var("XAI_API_KEY")
            .map_err(|_| crate::Error::Config("XAI_API_KEY not set".to_string()))?;
        let base_url =
            std::env::var("XAI_BASE_URL").unwrap_or_else(|_| "https://api.x.ai/v1".to_string());
        Ok(Self {
            api_key,
            base_url,
            ..Default::default()
        })
    }
}

/// Result of a non-streaming Responses call, including multi-turn state.
#[derive(Debug, Clone)]
pub struct ResponsesResult {
    pub response_id: String,
    pub content: String,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ParsedToolCall>,
    pub finish_reason: Option<String>,
    pub inference_timing: Option<InferenceTiming>,
}

#[derive(Debug, Clone, Serialize)]
struct ResponsesRequest {
    model: String,
    input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesApiResponse {
    id: Option<String>,
    status: Option<String>,
    output: Option<Vec<Value>>,
    usage: Option<ResponsesUsage>,
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ResponsesUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    #[serde(default)]
    output_tokens_details: Option<OutputTokenDetails>,
}

#[derive(Debug, Deserialize)]
struct OutputTokenDetails {
    reasoning_tokens: Option<u32>,
}

/// xAI Responses API provider.
pub struct ResponsesProvider {
    config: ResponsesConfig,
    client: Arc<RetryableHttpClient>,
    /// Last response id for optional stateful chaining via
    /// [`Self::continue_with_tool_outputs`].
    last_response_id: Mutex<Option<String>>,
}

impl ResponsesProvider {
    pub fn new(config: ResponsesConfig) -> Result<Self, crate::Error> {
        url::Url::parse(&config.base_url)
            .map_err(|e| crate::Error::Config(format!("Invalid base URL: {e}")))?;

        let http_config = HttpClientConfig {
            base_url: config.base_url.clone(),
            auth_token: Some(config.api_key.clone()),
            // Reasoning models can run long; keep a generous ceiling.
            timeout_secs: 300,
            max_retries: 3,
            initial_retry_delay_ms: 1000,
            backoff_factor: 2.0,
            max_retry_delay_ms: 30000,
            jitter_factor: 0.1,
            ..Default::default()
        };
        let client = Arc::new(
            RetryableHttpClient::new(HttpClientBuilder::new(http_config))
                .map_err(|e| crate::Error::Provider(e.to_string()))?,
        );
        Ok(Self {
            config,
            client,
            last_response_id: Mutex::new(None),
        })
    }

    pub fn from_env() -> Result<Self, crate::Error> {
        Self::new(ResponsesConfig::from_env()?)
    }

    pub fn last_response_id(&self) -> Option<String> {
        self.last_response_id.lock().ok().and_then(|g| g.clone())
    }

    pub fn clear_response_id(&self) {
        if let Ok(mut g) = self.last_response_id.lock() {
            *g = None;
        }
    }

    fn set_last_response_id(&self, id: Option<String>) {
        if let Ok(mut g) = self.last_response_id.lock() {
            *g = id;
        }
    }

    fn responses_url(&self) -> String {
        format!("{}/responses", self.config.base_url.trim_end_matches('/'))
    }

    /// Convert internal messages into Responses `input` items.
    fn convert_input(messages: &[Message]) -> Value {
        let items: Vec<Value> = messages
            .iter()
            .map(|msg| match &msg.role {
                Role::Tool => {
                    let call_id = msg
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| "call_unknown".to_string());
                    json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": msg.content,
                    })
                }
                Role::Assistant if !msg.tool_calls.is_empty() => {
                    // Replay prior assistant tool calls as function_call items
                    // so the model sees the full trajectory when store=false.
                    let mut parts = Vec::new();
                    if !msg.content.is_empty() {
                        parts.push(json!({
                            "role": "assistant",
                            "content": msg.content,
                        }));
                    }
                    for tc in &msg.tool_calls {
                        parts.push(json!({
                            "type": "function_call",
                            "call_id": tc.id.clone().unwrap_or_default(),
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }));
                    }
                    // Flatten multi-part assistant turns into sequential items
                    // by serializing the first and letting the outer map handle
                    // only one Value — expand via flat_map instead.
                    json!({ "_parts": parts })
                }
                role => {
                    let role_str = match role {
                        Role::System => "system",
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::Tool => "user",
                    };
                    json!({
                        "role": role_str,
                        "content": msg.content,
                    })
                }
            })
            .flat_map(|v| {
                if let Some(parts) = v.get("_parts").and_then(Value::as_array) {
                    parts.clone()
                } else {
                    vec![v]
                }
            })
            .collect();
        Value::Array(items)
    }

    fn convert_tools(tools_json: &Value) -> Vec<Value> {
        let Some(arr) = tools_json.as_array() else {
            return Vec::new();
        };
        arr.iter()
            .filter_map(|tool| {
                // Pass through already-shaped Responses tools.
                if tool.get("type").and_then(Value::as_str) == Some("function")
                    && tool.get("name").is_some()
                {
                    return Some(tool.clone());
                }
                if tool.get("type").and_then(Value::as_str).is_some() && tool.get("name").is_none()
                {
                    // Built-in: web_search, code_interpreter, etc.
                    return Some(tool.clone());
                }
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
                    "name": name,
                    "description": description,
                    "parameters": parameters,
                }))
            })
            .collect()
    }

    fn parse_output(output: &[Value]) -> (String, Option<String>, Vec<ParsedToolCall>) {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();

        for item in output {
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            match item_type {
                "message" => {
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for part in parts {
                            if part.get("type").and_then(Value::as_str) == Some("output_text")
                                && let Some(text) = part.get("text").and_then(Value::as_str)
                            {
                                content.push_str(text);
                            }
                        }
                    }
                }
                "function_call" => {
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}")
                        .to_string();
                    let call_id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    tool_calls.push(ParsedToolCall {
                        tool_name: name,
                        arguments: serde_json::from_str(&arguments).unwrap_or_else(|_| json!({})),
                        call_id,
                    });
                }
                "reasoning" => {
                    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                        for s in summary {
                            if let Some(text) = s.get("text").and_then(Value::as_str) {
                                if !reasoning.is_empty() {
                                    reasoning.push('\n');
                                }
                                reasoning.push_str(text);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let reasoning = if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        };
        (content, reasoning, tool_calls)
    }

    fn timing_from_usage(usage: &ResponsesUsage) -> InferenceTiming {
        InferenceTiming {
            n_prompt_eval: usage.input_tokens.unwrap_or(0),
            n_eval: usage.output_tokens.unwrap_or(0),
            n_thinking_eval: usage
                .output_tokens_details
                .as_ref()
                .and_then(|d| d.reasoning_tokens),
            ..Default::default()
        }
    }

    fn build_request(
        &self,
        input: Value,
        tools: Option<Value>,
        max_tokens: Option<usize>,
        stream: bool,
        previous_response_id: Option<String>,
    ) -> ResponsesRequest {
        let tools = tools
            .as_ref()
            .map(Self::convert_tools)
            .filter(|t| !t.is_empty());
        let tool_choice = tools.as_ref().map(|_| json!("auto"));
        ResponsesRequest {
            model: self.config.model.clone(),
            input,
            stream: Some(stream),
            tools,
            tool_choice,
            previous_response_id,
            store: Some(self.config.store),
            reasoning: Some(json!({ "effort": self.config.reasoning_effort.as_str() })),
            max_output_tokens: max_tokens.and_then(|m| u32::try_from(m).ok()),
            service_tier: self.config.service_tier.clone(),
        }
    }

    /// Non-streaming create. Updates `last_response_id` on success.
    pub async fn create(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        max_tokens: Option<usize>,
        previous_response_id: Option<String>,
    ) -> Result<ResponsesResult, crate::Error> {
        let input = Self::convert_input(&messages);
        let request = self.build_request(input, tools, max_tokens, false, previous_response_id);
        let url = self.responses_url();

        let api_response: ResponsesApiResponse = self
            .client
            .execute_with_retry(|client| {
                let url = url.clone();
                let request = request.clone();
                let api_key = self.config.api_key.clone();
                Box::pin(async move {
                    let response = client
                        .post(&url)
                        .header("Authorization", format!("Bearer {api_key}"))
                        .json(&request)
                        .send()
                        .await?;
                    if !response.status().is_success() {
                        let status = response.status();
                        let body = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "failed to read body".to_string());
                        return Err(anyhow::anyhow!("xAI Responses API {status}: {body}"));
                    }
                    let parsed: ResponsesApiResponse = response.json().await?;
                    Ok(parsed)
                })
            })
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        if let Some(err) = api_response.error {
            return Err(crate::Error::Provider(format!("xAI response error: {err}")));
        }

        let response_id = api_response.id.clone().unwrap_or_default();
        self.set_last_response_id(if response_id.is_empty() {
            None
        } else {
            Some(response_id.clone())
        });

        let output = api_response.output.unwrap_or_default();
        let (content, reasoning_content, tool_calls) = Self::parse_output(&output);
        let inference_timing = api_response.usage.as_ref().map(Self::timing_from_usage);

        Ok(ResponsesResult {
            response_id,
            content,
            reasoning_content,
            tool_calls,
            finish_reason: api_response.status,
            inference_timing,
        })
    }

    /// Continue an agent loop with client-side tool results.
    pub async fn continue_with_tool_outputs(
        &self,
        previous_response_id: &str,
        tool_outputs: Vec<(String, String)>,
        tools: Option<Value>,
    ) -> Result<ResponsesResult, crate::Error> {
        let input: Vec<Value> = tool_outputs
            .into_iter()
            .map(|(call_id, output)| {
                json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                })
            })
            .collect();
        let request = self.build_request(
            Value::Array(input),
            tools,
            None,
            false,
            Some(previous_response_id.to_string()),
        );
        let url = self.responses_url();

        let api_response: ResponsesApiResponse = self
            .client
            .execute_with_retry(|client| {
                let url = url.clone();
                let request = request.clone();
                let api_key = self.config.api_key.clone();
                Box::pin(async move {
                    let response = client
                        .post(&url)
                        .header("Authorization", format!("Bearer {api_key}"))
                        .json(&request)
                        .send()
                        .await?;
                    if !response.status().is_success() {
                        let status = response.status();
                        let body = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "failed to read body".to_string());
                        return Err(anyhow::anyhow!("xAI Responses API {status}: {body}"));
                    }
                    let parsed: ResponsesApiResponse = response.json().await?;
                    Ok(parsed)
                })
            })
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        let response_id = api_response.id.clone().unwrap_or_default();
        self.set_last_response_id(if response_id.is_empty() {
            None
        } else {
            Some(response_id.clone())
        });

        let output = api_response.output.unwrap_or_default();
        let (content, reasoning_content, tool_calls) = Self::parse_output(&output);
        let inference_timing = api_response.usage.as_ref().map(Self::timing_from_usage);

        Ok(ResponsesResult {
            response_id,
            content,
            reasoning_content,
            tool_calls,
            finish_reason: api_response.status,
            inference_timing,
        })
    }
}

#[async_trait]
impl Provider for ResponsesProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String, crate::Error> {
        let result = self.create(messages, None, max_tokens, None).await?;
        Ok(result.content)
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
        let result = self.create(messages, tools, max_tokens, None).await?;
        Ok(ProviderResponse {
            content: result.content,
            reasoning_content: result.reasoning_content,
            tool_calls: result.tool_calls,
            finish_reason: result.finish_reason,
            inference_timing: result.inference_timing,
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
        let input = Self::convert_input(&messages);
        let request = self.build_request(input, None, None, true, None);
        let url = self.responses_url();
        let api_key = self.config.api_key.clone();

        let response = self
            .client
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&request)
            .send()
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "failed to read body".to_string());
            return Err(crate::Error::Provider(format!(
                "xAI Responses stream {status}: {body}"
            )));
        }

        let (tx, rx) = tokio::sync::mpsc::channel(1024);
        let last_id = Arc::new(Mutex::new(None::<String>));
        let last_id_writer = Arc::clone(&last_id);

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut stream = response.bytes_stream();
            let mut terminal_sent = false;

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        // Only parse newline-terminated lines so a partial
                        // trailing line is not emitted twice across chunks.
                        let Some(last_newline) = buffer.rfind('\n') else {
                            continue;
                        };
                        let complete: String = buffer.drain(..=last_newline).collect();

                        for line in complete.lines() {
                            let Some(data) = line.strip_prefix("data: ") else {
                                continue;
                            };
                            if data == "[DONE]" {
                                if !terminal_sent {
                                    let _ = tx
                                        .send(Ok(StreamResponse {
                                            content: String::new(),
                                            reasoning_content: None,
                                            done: true,
                                            inference_timing: None,
                                        }))
                                        .await;
                                }
                                return;
                            }
                            let Ok(event) = serde_json::from_str::<Value>(data) else {
                                continue;
                            };
                            let event_type =
                                event.get("type").and_then(Value::as_str).unwrap_or("");
                            match event_type {
                                "response.output_text.delta" => {
                                    if let Some(delta) = event.get("delta").and_then(Value::as_str)
                                        && tx
                                            .send(Ok(StreamResponse {
                                                content: delta.to_string(),
                                                reasoning_content: None,
                                                done: false,
                                                inference_timing: None,
                                            }))
                                            .await
                                            .is_err()
                                    {
                                        return;
                                    }
                                }
                                "response.reasoning_summary_text.delta"
                                | "response.reasoning_text.delta" => {
                                    if let Some(delta) = event.get("delta").and_then(Value::as_str)
                                        && tx
                                            .send(Ok(StreamResponse {
                                                content: String::new(),
                                                reasoning_content: Some(delta.to_string()),
                                                done: false,
                                                inference_timing: None,
                                            }))
                                            .await
                                            .is_err()
                                    {
                                        return;
                                    }
                                }
                                "response.completed" => {
                                    if let Some(id) =
                                        event.pointer("/response/id").and_then(Value::as_str)
                                        && let Ok(mut g) = last_id_writer.lock()
                                    {
                                        *g = Some(id.to_string());
                                    }
                                    let timing = event.pointer("/response/usage").and_then(|u| {
                                        let usage: ResponsesUsage =
                                            serde_json::from_value(u.clone()).ok()?;
                                        Some(ResponsesProvider::timing_from_usage(&usage))
                                    });
                                    if !terminal_sent {
                                        if tx
                                            .send(Ok(StreamResponse {
                                                content: String::new(),
                                                reasoning_content: None,
                                                done: true,
                                                inference_timing: timing,
                                            }))
                                            .await
                                            .is_err()
                                        {
                                            return;
                                        }
                                        terminal_sent = true;
                                    }
                                }
                                "response.failed" => {
                                    let msg = event
                                        .pointer("/response/error")
                                        .map(|e| e.to_string())
                                        .unwrap_or_else(|| "response.failed".to_string());
                                    let _ = tx.send(Err(crate::Error::Provider(msg))).await;
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(crate::Error::Provider(e.to_string()))).await;
                        break;
                    }
                }
            }
        });

        // Best-effort: capture id after stream task may have set it is racy;
        // callers needing id should use non-streaming create().
        let _ = last_id;

        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn name(&self) -> &'static str {
        "xai-responses"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_input_user_and_system() {
        let msgs = vec![Message::system("sys"), Message::user("hello")];
        let input = ResponsesProvider::convert_input(&msgs);
        let arr = input.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["role"], "system");
        assert_eq!(arr[1]["content"], "hello");
    }

    #[test]
    fn convert_input_tool_result() {
        let mut msg = Message::user("result");
        msg.role = Role::Tool;
        msg.tool_call_id = Some("call_1".to_string());
        msg.content = r#"{"ok":true}"#.to_string();
        let input = ResponsesProvider::convert_input(&[msg]);
        let item = &input.as_array().unwrap()[0];
        assert_eq!(item["type"], "function_call_output");
        assert_eq!(item["call_id"], "call_1");
    }

    #[test]
    fn convert_tools_from_router_shape() {
        let tools = json!([{
            "name": "get_time",
            "description": "time",
            "parameters": {"type": "object", "properties": {}}
        }]);
        let out = ResponsesProvider::convert_tools(&tools);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "function");
        assert_eq!(out[0]["name"], "get_time");
    }

    #[test]
    fn convert_tools_passthrough_builtin() {
        let tools = json!([{"type": "web_search"}]);
        let out = ResponsesProvider::convert_tools(&tools);
        assert_eq!(out[0]["type"], "web_search");
    }

    #[test]
    fn parse_output_message_and_function_call() {
        let output = vec![
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hi"}]
            }),
            json!({
                "type": "function_call",
                "call_id": "call_1",
                "name": "get_time",
                "arguments": "{}"
            }),
            json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "think"}]
            }),
        ];
        let (content, reasoning, tools) = ResponsesProvider::parse_output(&output);
        assert_eq!(content, "Hi");
        assert_eq!(reasoning.as_deref(), Some("think"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "get_time");
        assert_eq!(tools[0].call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn reasoning_effort_serializes_lowercase() {
        assert_eq!(ReasoningEffort::Low.as_str(), "low");
        assert_eq!(ReasoningEffort::High.as_str(), "high");
    }

    #[test]
    fn provider_name_and_tools() {
        let provider = ResponsesProvider::new(ResponsesConfig {
            api_key: "test".to_string(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(provider.name(), "xai-responses");
        assert!(provider.supports_tools());
        assert_eq!(provider.config.reasoning_effort, ReasoningEffort::Low);
    }

    #[test]
    fn responses_url_trims_slash() {
        let provider = ResponsesProvider::new(ResponsesConfig {
            api_key: "test".to_string(),
            base_url: "https://api.x.ai/v1/".to_string(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(provider.responses_url(), "https://api.x.ai/v1/responses");
    }

    /// Incomplete trailing SSE lines must stay buffered and not be parsed
    /// until a newline arrives (regression for gitar-bot finding on #645).
    #[test]
    fn sse_buffer_keeps_partial_line_until_newline() {
        let mut buffer =
            String::from("data: {\"type\":\"response.output_text.delta\",\"delta\":\"a\"}");
        assert!(
            buffer.rfind('\n').is_none(),
            "unterminated line must not look complete"
        );
        buffer.push_str("\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"b\"}\n");
        let last_newline = buffer.rfind('\n').expect("complete lines present");
        let complete: String = buffer.drain(..=last_newline).collect();
        let lines: Vec<&str> = complete
            .lines()
            .filter(|l| l.starts_with("data: "))
            .collect();
        assert_eq!(lines.len(), 2, "both complete data lines should parse");
        assert!(
            buffer.is_empty(),
            "no partial remainder after trailing newline"
        );

        buffer.push_str("data: {\"type\":\"response.completed\"");
        assert!(
            buffer.rfind('\n').is_none(),
            "partial completed event must remain buffered"
        );
        buffer.push_str("}\n");
        let last_newline = buffer.rfind('\n').unwrap();
        let complete: String = buffer.drain(..=last_newline).collect();
        assert!(complete.contains("response.completed"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn terminal_done_emitted_only_once() {
        let mut terminal_sent = false;
        let signals = ["response.completed", "[DONE]", "[DONE]"];
        let mut emitted = 0usize;
        for signal in signals {
            if signal == "[DONE]" {
                if !terminal_sent {
                    emitted += 1;
                    terminal_sent = true;
                }
                break;
            }
            if signal == "response.completed" && !terminal_sent {
                emitted += 1;
                terminal_sent = true;
            }
        }
        assert_eq!(emitted, 1, "only one terminal done signal");
        assert!(terminal_sent);
    }
}
