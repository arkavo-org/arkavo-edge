use super::config::ResponsesConfig;
use super::convert::{convert_input, convert_tools, parse_output};
use super::sse::{
    SseAction, action_sets_terminal, append_utf8_chunk, drain_complete_sse_lines,
    handle_sse_data_line, should_stop_after,
};
use super::types::{ResponsesApiResponse, ResponsesRequest, ResponsesResult, timing_from_usage};
use crate::common::{HttpClientBuilder, HttpClientConfig, RetryableHttpClient};
use crate::provider::ProviderResponse;
use crate::{Message, Provider, StreamResponse};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::Mutex;

/// xAI Responses API provider (`POST /v1/responses`).
///
/// ## Multi-turn (v1)
///
/// `Provider::complete_with_tools` always re-sends the full message transcript
/// with `previous_response_id: None`. Stateful chaining is available via
/// [`Self::continue_with_tool_outputs`] when `store` is enabled and the caller
/// tracks response ids. Streaming updates [`Self::last_response_id`] when the
/// `response.completed` event carries an id.
pub struct ResponsesProvider {
    config: ResponsesConfig,
    client: Arc<RetryableHttpClient>,
    last_response_id: Arc<Mutex<Option<String>>>,
}

impl ResponsesProvider {
    pub fn new(config: ResponsesConfig) -> Result<Self, crate::Error> {
        url::Url::parse(&config.base_url)
            .map_err(|e| crate::Error::Config(format!("Invalid base URL: {e}")))?;

        let http_config = HttpClientConfig {
            base_url: config.base_url.clone(),
            auth_token: Some(config.api_key.clone()),
            timeout_secs: config.reasoning_effort.request_timeout_secs(),
            max_retries: config.reasoning_effort.max_retries(),
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
            last_response_id: Arc::new(Mutex::new(None)),
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

    fn build_request(
        &self,
        input: Value,
        tools: Option<Value>,
        max_tokens: Option<usize>,
        stream: bool,
        previous_response_id: Option<String>,
    ) -> ResponsesRequest {
        let tools = tools.as_ref().map(convert_tools).filter(|t| !t.is_empty());
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
            prompt_cache_key: self.config.prompt_cache_key.clone(),
        }
    }

    async fn post_responses(
        &self,
        request: ResponsesRequest,
    ) -> Result<ResponsesApiResponse, crate::Error> {
        let url = self.responses_url();
        self.client
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
            .map_err(|e| crate::Error::Provider(e.to_string()))
    }

    fn result_from_api(
        &self,
        api_response: ResponsesApiResponse,
    ) -> Result<ResponsesResult, crate::Error> {
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
        let (content, reasoning_content, tool_calls) = parse_output(&output);
        let inference_timing = api_response.usage.as_ref().map(timing_from_usage);

        Ok(ResponsesResult {
            response_id,
            content,
            reasoning_content,
            tool_calls,
            // Responses `status` (e.g. "completed"), not OpenAI finish_reason.
            finish_reason: api_response.status,
            inference_timing,
        })
    }

    /// Non-streaming create. Updates `last_response_id` on success.
    pub async fn create(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        max_tokens: Option<usize>,
        previous_response_id: Option<String>,
    ) -> Result<ResponsesResult, crate::Error> {
        let input = convert_input(&messages);
        let request = self.build_request(input, tools, max_tokens, false, previous_response_id);
        let api_response = self.post_responses(request).await?;
        self.result_from_api(api_response)
    }

    /// Continue an agent loop with client-side tool results (requires `store`).
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
        let api_response = self.post_responses(request).await?;
        self.result_from_api(api_response)
    }
}

#[async_trait]
impl Provider for ResponsesProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String, crate::Error> {
        // v1: full-transcript multi-turn (previous_response_id always None).
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
        // v1: full-transcript multi-turn (previous_response_id always None).
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
        let input = convert_input(&messages);
        let request = self.build_request(input, None, None, true, None);
        let url = self.responses_url();
        let api_key = self.config.api_key.clone();
        let last_response_id = Arc::clone(&self.last_response_id);

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

        tokio::spawn(async move {
            // Byte pending retains incomplete multi-byte UTF-8 sequences that
            // straddle TCP chunks; text buffer only grows with valid UTF-8.
            let mut pending_utf8 = Vec::new();
            let mut buffer = String::new();
            let mut stream = response.bytes_stream();
            let mut terminal_sent = false;

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        append_utf8_chunk(&mut pending_utf8, &mut buffer, &bytes);
                        let Some(complete) = drain_complete_sse_lines(&mut buffer) else {
                            continue;
                        };

                        for line in complete.lines() {
                            let Some(data) = line.strip_prefix("data: ") else {
                                continue;
                            };
                            let action = handle_sse_data_line(data, terminal_sent, &mut |id| {
                                if let Ok(mut g) = last_response_id.lock() {
                                    *g = Some(id);
                                }
                            });

                            let stop = should_stop_after(&action, data);
                            if action_sets_terminal(&action) {
                                terminal_sent = true;
                            }
                            match action {
                                SseAction::Emit(chunk) => {
                                    if tx.send(Ok(chunk)).await.is_err() {
                                        return;
                                    }
                                }
                                SseAction::Fail(msg) => {
                                    let _ = tx.send(Err(crate::Error::Provider(msg))).await;
                                    return;
                                }
                                SseAction::Finished | SseAction::Ignore => {}
                            }

                            if stop {
                                return;
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

        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn name(&self) -> &'static str {
        "xai-responses"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::xai_responses::config::ReasoningEffort;

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
        assert!(!provider.config.store);
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

    #[test]
    fn build_request_includes_prompt_cache_key_and_store() {
        let provider = ResponsesProvider::new(ResponsesConfig {
            api_key: "test".to_string(),
            store: true,
            prompt_cache_key: Some("session-abc".into()),
            ..Default::default()
        })
        .unwrap();
        let req = provider.build_request(json!([]), None, Some(64), false, None);
        assert_eq!(req.store, Some(true));
        assert_eq!(req.prompt_cache_key.as_deref(), Some("session-abc"));
        assert_eq!(req.max_output_tokens, Some(64));
    }

    #[test]
    fn build_request_serializes_xhigh_effort() {
        let provider = ResponsesProvider::new(ResponsesConfig {
            api_key: "test".to_string(),
            model: "grok-4.6".to_string(),
            reasoning_effort: ReasoningEffort::Xhigh,
            ..Default::default()
        })
        .unwrap();
        let req = provider.build_request(json!([]), None, None, false, None);
        assert_eq!(req.model, "grok-4.6");
        assert_eq!(req.reasoning.unwrap()["effort"], "xhigh");
    }
}
