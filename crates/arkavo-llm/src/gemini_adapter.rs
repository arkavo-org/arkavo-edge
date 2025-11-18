use crate::error::{Error, Result};
use crate::message::Message;
use crate::provider::{Provider, ProviderResponse};
use crate::stream::StreamResponse;
use crate::tool_parser::ParsedToolCall;
use arkavo_gemini::{FunctionDeclaration, GeminiSseStream, RestClient};
use async_trait::async_trait;
use serde_json::Value;
use std::env;
use tokio_stream::StreamExt;

pub struct GeminiProvider {
    client: RestClient,
}

impl GeminiProvider {
    pub fn new() -> Result<Self> {
        let api_key = env::var("GEMINI_API_KEY").map_err(|_| {
            Error::Config("GEMINI_API_KEY not set (optional - will fallback to local model)".into())
        })?;

        let model = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-latest".to_string());

        Ok(Self {
            client: RestClient::new(api_key, model),
        })
    }

    /// Try to create a Gemini provider, returning None if API key is not available
    pub fn try_new() -> Option<Self> {
        Self::new().ok()
    }
}

impl Default for GeminiProvider {
    fn default() -> Self {
        Self::new().expect("Failed to create GeminiProvider")
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "gemini"
    }

    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        if messages.is_empty() {
            return Err(Error::Provider("No messages provided".into()));
        }

        let last_message = messages
            .last()
            .ok_or_else(|| Error::Provider("No messages provided".into()))?;

        let prompt = &last_message.content;

        let mut gemini_stream = self
            .client
            .stream_generate_content(prompt, None)
            .await
            .map_err(|e| Error::Provider(format!("Gemini streaming error: {e}")))?;

        let mut accumulated_text = String::new();

        while let Some(chunk_result) = gemini_stream.next().await {
            let chunk = chunk_result.map_err(|e| Error::Stream(format!("Stream error: {e}")))?;

            if let Some(text) = chunk.text {
                accumulated_text.push_str(&text);
            }

            if chunk.done {
                break;
            }
        }

        if accumulated_text.is_empty() {
            return Err(Error::Provider("No text response from Gemini".into()));
        }

        Ok(accumulated_text)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn tokio_stream::Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        if messages.is_empty() {
            return Err(Error::Provider("No messages provided".into()));
        }

        let last_message = messages
            .last()
            .ok_or_else(|| Error::Provider("No messages provided".into()))?;

        let prompt = &last_message.content;

        let gemini_stream: GeminiSseStream = self
            .client
            .stream_generate_content(prompt, None)
            .await
            .map_err(|e| Error::Stream(format!("Gemini streaming error: {e}")))?;

        let adapter_stream = gemini_stream.map(|result| {
            result
                .map(|response| StreamResponse {
                    content: response.text.unwrap_or_default(),
                    done: response.done,
                })
                .map_err(|e| Error::Stream(format!("Stream error: {e}")))
        });

        Ok(Box::new(Box::pin(adapter_stream)))
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        _max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        if messages.is_empty() {
            return Err(Error::Provider("No messages provided".into()));
        }

        let last_message = messages
            .last()
            .ok_or_else(|| Error::Provider("No messages provided".into()))?;

        let prompt = &last_message.content;

        let tool_declarations = tools.and_then(|t| Self::convert_tools_to_declarations(&t).ok());

        let mut gemini_stream = self
            .client
            .stream_generate_content(prompt, tool_declarations)
            .await
            .map_err(|e| Error::Provider(format!("Gemini streaming error: {e}")))?;

        let mut accumulated_text = String::new();
        let mut all_function_calls = Vec::new();

        while let Some(chunk_result) = gemini_stream.next().await {
            let chunk = chunk_result.map_err(|e| Error::Stream(format!("Stream error: {e}")))?;

            if let Some(text) = chunk.text {
                accumulated_text.push_str(&text);
            }

            if !chunk.function_calls.is_empty() {
                all_function_calls.extend(chunk.function_calls);
            }

            if chunk.done {
                break;
            }
        }

        let parsed_tool_calls = all_function_calls
            .into_iter()
            .map(|fc| ParsedToolCall {
                tool_name: fc.name,
                arguments: fc.args,
                call_id: Some(fc.id),
            })
            .collect();

        Ok(ProviderResponse {
            content: accumulated_text,
            tool_calls: parsed_tool_calls,
            finish_reason: None,
        })
    }
}

impl GeminiProvider {
    fn convert_tools_to_declarations(tools_json: &Value) -> Result<Vec<FunctionDeclaration>> {
        let tools_array = tools_json
            .as_array()
            .ok_or_else(|| Error::Provider("Tools must be an array".into()))?;

        if tools_array.is_empty() {
            return Ok(Vec::new());
        }

        let first_tool = &tools_array[0];
        let function_declarations = first_tool
            .get("functionDeclarations")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Provider("Invalid Gemini tools format".into()))?;

        function_declarations
            .iter()
            .map(|decl| {
                Ok(FunctionDeclaration {
                    name: decl
                        .get("name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| Error::Provider("Tool missing name".into()))?
                        .to_string(),
                    description: decl
                        .get("description")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| Error::Provider("Tool missing description".into()))?
                        .to_string(),
                    parameters: decl
                        .get("parameters")
                        .cloned()
                        .ok_or_else(|| Error::Provider("Tool missing parameters".into()))?,
                })
            })
            .collect()
    }
}
