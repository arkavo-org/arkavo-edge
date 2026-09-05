use crate::config::LlmConfig;
use crate::error::{Error, Result};
use crate::mcp_converter::McpConverter;
use crate::message::{Message, Role};
use crate::provider::{InferenceTiming, Provider, ProviderResponse};
use crate::stream::StreamResponse;
use crate::tool_parser::ParsedToolCall;
use arkavo_gemini::{
    FunctionDeclaration, GeminiSseStream, RestClient, ThinkingConfig, UsageMetadata,
};
use async_trait::async_trait;
use serde_json::Value;
use std::env;
use tokio_stream::StreamExt;

pub struct GeminiProvider {
    client: RestClient,
    default_thinking: Option<ThinkingConfig>,
}

impl GeminiProvider {
    pub fn new() -> Result<Self> {
        let api_key = env::var("GEMINI_API_KEY").map_err(|_| {
            Error::Config("GEMINI_API_KEY not set (optional - will fallback to local model)".into())
        })?;

        let model = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.5-flash".to_string());

        Ok(Self {
            client: RestClient::new(api_key, model),
            default_thinking: Some(Self::default_thinking_from_env()),
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
            default_thinking: Some(Self::default_thinking_from_env()),
        })
    }

    /// Override the default thinking budget. `None` falls back to server
    /// default (dynamic for Gemini 3.5 Flash) — only use when you have an
    /// explicit deliberation code path that benefits from extended reasoning.
    pub fn with_thinking(mut self, thinking: Option<ThinkingConfig>) -> Self {
        self.default_thinking = thinking;
        self
    }

    /// Construct a provider pinned to a specific model + thinking budget.
    /// Used by the router to materialise the distinct Thompson Sampling
    /// arms (`Gemini35FlashMinimal/Low/Medium/High`) — each arm shares the
    /// `gemini-3.5-flash` API model but ships its own `thinkingBudget`.
    pub fn for_model_with_thinking(
        model: impl Into<String>,
        thinking_budget: Option<i32>,
    ) -> Result<Self> {
        let api_key = env::var("GEMINI_API_KEY").map_err(|_| {
            Error::Config("GEMINI_API_KEY not set (optional - will fallback to local model)".into())
        })?;
        let default_thinking = thinking_budget.map(|b| ThinkingConfig {
            thinking_budget: Some(b),
            include_thoughts: None,
        });
        Ok(Self {
            client: RestClient::new(api_key, model),
            default_thinking,
        })
    }

    /// Production default: pin `thinkingBudget` to `512` (low) unless the
    /// `GEMINI_THINKING_BUDGET` env var overrides.
    ///
    /// Gemini 3.5 Flash defaults to dynamic thinking, which Artificial
    /// Analysis flagged as the dominant driver of bimodal latency (1–3s
    /// success → 90s timeouts when the model decides a task is "hard").
    /// We pin to the "low" tier — enough chain-of-thought budget for the
    /// model to commit to tool calls (with `0`/off it just emitted
    /// `observe` over and over in the rimworld loop instead of stepping)
    /// but capped well below the runaway-thinking regime.
    ///
    /// Env values: `0` (off), `-1` (dynamic), positive integer (exact
    /// token cap). Default if unset: `512`. Use `with_thinking` for
    /// explicit deliberation paths that want medium/high.
    fn default_thinking_from_env() -> ThinkingConfig {
        let budget = env::var("GEMINI_THINKING_BUDGET")
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(512);
        ThinkingConfig {
            thinking_budget: Some(budget),
            include_thoughts: None,
        }
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
            .stream_generate_content(prompt, None, self.default_thinking.clone())
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
            .stream_generate_content(prompt, None, self.default_thinking.clone())
            .await
            .map_err(|e| Error::Stream(format!("Gemini streaming error: {e}")))?;

        let adapter_stream = gemini_stream.map(|result| {
            result
                .map(|response| StreamResponse {
                    response_items: Vec::new(),
                    content: response.text.unwrap_or_default(),
                    // Surface Gemini 3.5 thought-summary segments via the
                    // standard reasoning-content channel.
                    reasoning_content: response.thought_text,
                    done: response.done,
                    // Token usage only arrives on the terminal chunk;
                    // intermediate deltas get `None`.
                    inference_timing: response.usage.as_ref().map(timing_from_usage),
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
            .stream_generate_content_json(prompt, schema, self.default_thinking.clone())
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
            .stream_generate_content_multi(
                system_instruction,
                contents,
                tool_declarations,
                self.default_thinking.clone(),
            )
            .await
            .map_err(|e| Error::Provider(format!("Gemini streaming error: {e}")))?;

        let mut accumulated_text = String::new();
        let mut accumulated_thought = String::new();
        let mut all_function_calls = Vec::new();
        let mut last_usage: Option<UsageMetadata> = None;

        while let Some(chunk_result) = gemini_stream.next().await {
            let chunk = chunk_result.map_err(|e| Error::Stream(format!("Stream error: {e}")))?;

            if let Some(text) = chunk.text {
                accumulated_text.push_str(&text);
            }
            if let Some(thought) = chunk.thought_text {
                accumulated_thought.push_str(&thought);
            }
            if !chunk.function_calls.is_empty() {
                all_function_calls.extend(chunk.function_calls);
            }
            if chunk.usage.is_some() {
                last_usage = chunk.usage;
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
            response_items: Vec::new(),
            content: accumulated_text,
            reasoning_content: if accumulated_thought.is_empty() {
                None
            } else {
                Some(accumulated_thought)
            },
            tool_calls: parsed_tool_calls,
            finish_reason: None,
            inference_timing: last_usage.as_ref().map(timing_from_usage),
            quality_gate_retries: 0,
        })
    }
}

/// Convert Gemini `usageMetadata` into the LLM-crate `InferenceTiming`
/// shape so the router/budget paths see token counts uniformly across
/// providers. Latencies stay zero for cloud calls (we don't get per-phase
/// timings from the API today), but token counts — input, output,
/// thinking — are the load-bearing fields for cost tracking.
fn timing_from_usage(usage: &UsageMetadata) -> InferenceTiming {
    InferenceTiming {
        n_cached_prompt_eval: None,
        n_cache_write_prompt_eval: None,
        prompt_eval_ms: 0.0,
        generation_ms: 0.0,
        n_prompt_eval: usage.prompt_token_count.unwrap_or(0),
        n_eval: usage.candidates_token_count.unwrap_or(0),
        n_thinking_eval: usage.thoughts_token_count,
        n_draft: None,
        n_accepted: None,
        spec_bypassed: None,
        avg_logprob: None,
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
                    // Gemini's `contents` array knows only user and model, so a
                    // tool result travels as user text that names its tool.
                    contents.push(("user".to_string(), msg.tool_result_as_user_text()));
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
