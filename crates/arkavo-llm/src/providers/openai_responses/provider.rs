use super::{OpenAIResponsesConfig, convert, sse};
use crate::{Error, Message, Provider, ProviderResponse, Result, StreamResponse};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use reqwest::{Client, Response};
use serde_json::Value;
use std::time::{Duration, Instant};

/// An error body is a diagnostic, not a payload: read a bounded prefix so a
/// broken or hostile endpoint cannot make the client buffer an unbounded reply.
const MAX_ERROR_BODY: usize = 64 * 1024;

/// How long an error body has to arrive before the status alone has to do.
const ERROR_BODY_BUDGET: Duration = Duration::from_secs(5);

pub struct OpenAIResponsesProvider {
    config: OpenAIResponsesConfig,
    client: Client,
    endpoint: String,
    api_key: String,
}

impl OpenAIResponsesProvider {
    pub fn new(config: OpenAIResponsesConfig) -> Result<Self> {
        config.validate()?;
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| Error::Config("OPENAI_API_KEY is required for GPT-6 Astra".into()))?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(30))
            // Deeper reasoning legitimately takes longer; a low-effort call
            // must not inherit the deepest tier's patience.
            .timeout(config.request_timeout())
            // Never forward credentials through an unexpected redirect.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let endpoint = format!("{}/responses", config.base_url.trim_end_matches('/'));
        Ok(Self {
            config,
            client,
            endpoint,
            api_key,
        })
    }

    async fn send(&self, body: Value) -> Result<Response> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Request(e.without_url()))?;
        if !response.status().is_success() {
            return Err(refusal(response).await);
        }
        Ok(response)
    }

    async fn complete_response(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        schema: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let body = convert::request(&self.config, messages, tools, schema, max_tokens, false)?;
        let started = Instant::now();
        let response = self.send(body).await?;
        let value = response
            .json()
            .await
            .map_err(|e| Error::Request(e.without_url()))?;
        let mut response = convert::response(value)?;
        if let Some(timing) = response.inference_timing.as_mut() {
            // A non-streamed reply exposes no first-token boundary, so the whole
            // wall time is attributed to generation rather than split on a guess.
            timing.generation_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        Ok(response)
    }
}

/// Turn a failed HTTP response into a refusal that names the provider's code.
///
/// The body's `message` is never read: it can quote the prompt or a credential.
/// `error.code`, then `error.type`, then the HTTP status supply the identifier.
async fn refusal(response: Response) -> Error {
    let fallback = format!("http_{}", response.status().as_u16());
    // A diagnostic is not worth waiting on: if the body stalls, the status
    // already names the failure, and the caller is already in an error path.
    let body = tokio::time::timeout(ERROR_BODY_BUDGET, error_body(response))
        .await
        .unwrap_or_default();
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let code = value.pointer("/error/code").and_then(Value::as_str);
    let kind = value.pointer("/error/type").and_then(Value::as_str);
    Error::provider_refusal(code.or(kind), kind, &fallback)
}

async fn error_body(response: Response) -> Vec<u8> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while body.len() < MAX_ERROR_BODY {
        match chunks.next().await {
            Some(Ok(chunk)) => {
                let room = MAX_ERROR_BODY - body.len();
                body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            }
            _ => break,
        }
    }
    body
}

#[async_trait]
impl Provider for OpenAIResponsesProvider {
    fn name(&self) -> &str {
        &self.config.model
    }
    fn supports_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        true
    }

    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        Ok(self
            .complete_response(messages, None, None, max_tokens)
            .await?
            .content)
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        self.complete_response(messages, tools, None, max_tokens)
            .await
    }

    async fn complete_with_schema(
        &self,
        messages: Vec<Message>,
        schema: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        Ok(self
            .complete_with_schema_response(messages, schema, max_tokens)
            .await?
            .content)
    }

    async fn complete_with_schema_response(
        &self,
        messages: Vec<Message>,
        schema: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        self.complete_response(messages, None, schema, max_tokens)
            .await
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let body = convert::request(&self.config, messages, None, None, None, true)?;
        let response = self.send(body).await?;
        Ok(sse::stream(response, self.config.stream_idle_timeout()))
    }
}
