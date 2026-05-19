use crate::config::LlmConfig;
use crate::error::{Error, Result};
use crate::mcp_converter::McpConverter;
use crate::message::{Message, Role};
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

        let model = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.5-flash".to_string());

        Ok(Self {
            client: RestClient::new(api_key, model),
        })
    }

    /// Try to create a Gemini provider, returning None if API key is not available
    pub fn try_new() -> Option<Self> {
        Self::new().ok()
    }

    /// Create from LlmConfig
    pub fn from_config(config: &LlmConfig) -> Result<Self> {
        let api_key = config
            .get_api_key("GEMINI_API_KEY")
            .ok_or_else(|| Error::Config("GEMINI_API_KEY not provided in config".to_string()))?;

        let model = config
            .model
            .clone()
            .unwrap_or_else(|| "gemini-3.5-flash".to_string());

        Ok(Self {
            client: RestClient::new(api_key.clone(), model),
        })
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
                    // Surface Gemini 3.5 thought-summary segments via the
                    // standard reasoning-content channel.
                    reasoning_content: response.thought_text,
                    done: response.done,
                    inference_timing: None,
                })
                .map_err(|e| Error::Stream(format!("Stream error: {e}")))
        });

        Ok(Box::new(Box::pin(adapter_stream)))
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_structured_output(&self) -> bool {
        true
    }

    async fn complete_with_schema(
        &self,
        messages: Vec<Message>,
        schema: Option<Value>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        if messages.is_empty() {
            return Err(Error::Provider("No messages provided".into()));
        }

        let last_message = messages
            .last()
            .ok_or_else(|| Error::Provider("No messages provided".into()))?;

        let prompt = &last_message.content;

        // If no schema provided, fall back to regular completion
        let Some(schema) = schema else {
            return self.complete_with_options(messages, None).await;
        };

        let mut gemini_stream = self
            .client
            .stream_generate_content_json(prompt, schema)
            .await
            .map_err(|e| Error::Provider(format!("Gemini JSON streaming error: {e}")))?;

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
            return Err(Error::Provider("No JSON response from Gemini".into()));
        }

        Ok(accumulated_text)
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

        let tool_declarations = tools.and_then(|t| Self::convert_tools_to_declarations(&t).ok());

        // Extract system instruction and convert messages to multi-turn contents
        let (system_instruction, contents) = Self::convert_messages_to_contents(&messages);

        let mut gemini_stream = self
            .client
            .stream_generate_content_multi(system_instruction, contents, tool_declarations)
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
            reasoning_content: None,
            tool_calls: parsed_tool_calls,
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        })
    }
}

impl GeminiProvider {
    /// Convert arkavo-llm Messages to Gemini API format.
    /// System messages become system_instruction; user/assistant/tool become contents.
    fn convert_messages_to_contents(
        messages: &[Message],
    ) -> (Option<String>, Vec<(String, String)>) {
        let mut system_parts = Vec::new();
        let mut contents = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    system_parts.push(msg.content.clone());
                }
                Role::User => {
                    contents.push(("user".to_string(), msg.content.clone()));
                }
                Role::Assistant => {
                    if !msg.content.is_empty() {
                        contents.push(("model".to_string(), msg.content.clone()));
                    }
                }
                Role::Tool => {
                    // Tool results go as user messages with context
                    let tool_text = if let Some(name) = &msg.tool_name {
                        format!("[Tool result from {}]: {}", name, msg.content)
                    } else {
                        msg.content.clone()
                    };
                    contents.push(("user".to_string(), tool_text));
                }
            }
        }

        let system_instruction = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };

        // Gemini requires at least one content entry
        if contents.is_empty() {
            contents.push(("user".to_string(), "Continue.".to_string()));
        }

        (system_instruction, contents)
    }

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
                let raw_params = decl
                    .get("parameters")
                    .cloned()
                    .ok_or_else(|| Error::Provider("Tool missing parameters".into()))?;

                // Sanitize parameters to remove invalid fields (name, aliases, etc.)
                // that may leak from MCP ToolSchema and cause Gemini API errors
                let sanitized_params = McpConverter::make_gemini_compatible(&raw_params);

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
                    parameters: sanitized_params,
                })
            })
            .collect()
    }
}
