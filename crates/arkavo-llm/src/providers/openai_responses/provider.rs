use super::{OpenAIResponsesConfig, convert, sse};
use crate::{Error, Message, Provider, ProviderResponse, Result, StreamResponse};
use async_trait::async_trait;
use futures::Stream;
use reqwest::{Client, Response};
use serde_json::Value;
use std::time::Duration;

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
            .timeout(Duration::from_mins(15))
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
            // API bodies can echo prompts or credentials; status is safe to report.
            return Err(Error::Provider(format!(
                "OpenAI Responses HTTP {}",
                response.status().as_u16()
            )));
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
        let response = self.send(body).await?;
        let value = response
            .json()
            .await
            .map_err(|e| Error::Request(e.without_url()))?;
        convert::response(value)
    }
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
        Ok(sse::stream(response))
    }
}
