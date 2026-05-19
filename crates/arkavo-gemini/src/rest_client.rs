use crate::error::{GeminiError, Result};
use crate::sse_stream::GeminiSseStream;
use crate::types::{FunctionCall, FunctionDeclaration, ListModelsResponse, Tool};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const GEMINI_REST_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Debug, Clone)]
pub struct RestClient {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Debug, Clone, Serialize)]
struct GenerateContentRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    system_instruction: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "generationConfig")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Content {
    role: String,
    parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Part {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCallPart,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: FunctionResponsePart,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionCallPart {
    name: String,
    args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionResponsePart {
    name: String,
    response: Value,
}

#[derive(Debug, Clone, Default, Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxOutputTokens")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "responseMimeType")]
    response_mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "responseSchema")]
    response_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "thinkingConfig")]
    thinking_config: Option<ThinkingConfig>,
}

/// Gemini 3.5 Thinking controls.
///
/// `thinking_budget` clamps the internal reasoning loop:
/// - `Some(0)`     — disable thinking (fastest, cheapest).
/// - `Some(-1)`    — dynamic; the model decides budget.
/// - `Some(n>0)`   — exact token cap for the chain-of-thought.
/// - `None`        — server default.
///
/// `include_thoughts: true` surfaces `thought=true` parts in the response,
/// which the stream parser folds into `StreamResponse::thought_text`.
///
/// Per https://ai.google.dev/gemini-api/docs/thinking
#[derive(Debug, Clone, Default, Serialize)]
pub struct ThinkingConfig {
    #[serde(skip_serializing_if = "Option::is_none", rename = "thinkingBudget")]
    pub thinking_budget: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "includeThoughts")]
    pub include_thoughts: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct GenerateContentResponse {
    candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Deserialize)]
struct Candidate {
    content: Content,
}

impl RestClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_mins(2))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    pub async fn generate_content(
        &self,
        prompt: impl Into<String>,
        tools: Option<Vec<FunctionDeclaration>>,
        thinking: Option<ThinkingConfig>,
    ) -> Result<(Option<String>, Vec<FunctionCall>)> {
        let generation_config = thinking.map(|t| GenerationConfig {
            thinking_config: Some(t),
            ..GenerationConfig::default()
        });
        let request = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![Part::Text {
                    text: prompt.into(),
                }],
            }],
            system_instruction: None,
            tools: tools.map(|t| {
                vec![Tool {
                    function_declarations: t,
                }]
            }),
            generation_config,
        };

        // Remove 'models/' prefix if present since we'll add it in the URL
        let model_name = self.model.strip_prefix("models/").unwrap_or(&self.model);
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            GEMINI_REST_ENDPOINT, model_name, self.api_key
        );

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| GeminiError::ApiError(format!("HTTP request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GeminiError::ApiError(format!(
                "HTTP {status}: {error_text}"
            )));
        }

        let result: GenerateContentResponse = response
            .json()
            .await
            .map_err(|e| GeminiError::ApiError(e.to_string()))?;

        let mut text_response = None;
        let mut function_calls = Vec::new();

        if let Some(candidate) = result.candidates.first() {
            for part in &candidate.content.parts {
                match part {
                    Part::Text { text } => {
                        text_response = Some(text.clone());
                    }
                    Part::FunctionCall { function_call } => {
                        function_calls.push(FunctionCall {
                            name: function_call.name.clone(),
                            args: function_call.args.clone(),
                            id: format!("call-{}", uuid::Uuid::new_v4()),
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok((text_response, function_calls))
    }

    /// Stream a single-turn prompt with optional tools and thinking budget.
    ///
    /// For Gemini 3.5+ models, `thinking` lets callers pin the
    /// `thinkingBudget` — pass `Some(ThinkingConfig { thinking_budget: Some(0), .. })`
    /// to disable dynamic chain-of-thought (recommended for latency-sensitive
    /// step-planning calls; the model default is dynamic, which the AA report
    /// flags as the primary cause of bimodal latency spikes).
    pub async fn stream_generate_content(
        &self,
        prompt: impl Into<String>,
        tools: Option<Vec<FunctionDeclaration>>,
        thinking: Option<ThinkingConfig>,
    ) -> Result<GeminiSseStream> {
        self.stream_generate_content_impl(prompt, tools, None, thinking)
            .await
    }

    pub async fn stream_generate_content_json(
        &self,
        prompt: impl Into<String>,
        schema: Value,
        thinking: Option<ThinkingConfig>,
    ) -> Result<GeminiSseStream> {
        self.stream_generate_content_impl(prompt, None, Some(schema), thinking)
            .await
    }

    /// Multi-turn stream with optional system instruction and thinking budget.
    /// This is the method `GeminiProvider` uses for tool-calling conversations.
    pub async fn stream_generate_content_multi(
        &self,
        system_instruction: Option<String>,
        contents: Vec<(String, String)>, // (role, text) pairs
        tools: Option<Vec<FunctionDeclaration>>,
        thinking: Option<ThinkingConfig>,
    ) -> Result<GeminiSseStream> {
        let sys = system_instruction.map(|text| Content {
            role: "user".to_string(), // Gemini system_instruction ignores role
            parts: vec![Part::Text { text }],
        });

        let contents = contents
            .into_iter()
            .map(|(role, text)| Content {
                role,
                parts: vec![Part::Text { text }],
            })
            .collect();

        let generation_config = thinking.map(|t| GenerationConfig {
            thinking_config: Some(t),
            ..GenerationConfig::default()
        });

        let request = GenerateContentRequest {
            contents,
            system_instruction: sys,
            tools: tools.map(|t| {
                vec![Tool {
                    function_declarations: t,
                }]
            }),
            generation_config,
        };

        self.stream_request(request).await
    }

    async fn stream_generate_content_impl(
        &self,
        prompt: impl Into<String>,
        tools: Option<Vec<FunctionDeclaration>>,
        json_schema: Option<Value>,
        thinking: Option<ThinkingConfig>,
    ) -> Result<GeminiSseStream> {
        let generation_config = match (json_schema, thinking) {
            (None, None) => None,
            (schema, thinking) => Some(GenerationConfig {
                temperature: None,
                max_output_tokens: None,
                response_mime_type: schema.as_ref().map(|_| "application/json".to_string()),
                response_schema: schema,
                thinking_config: thinking,
            }),
        };

        let request = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![Part::Text {
                    text: prompt.into(),
                }],
            }],
            system_instruction: None,
            tools: tools.map(|t| {
                vec![Tool {
                    function_declarations: t,
                }]
            }),
            generation_config,
        };

        self.stream_request(request).await
    }

    async fn stream_request(&self, request: GenerateContentRequest) -> Result<GeminiSseStream> {
        let model_name = self.model.strip_prefix("models/").unwrap_or(&self.model);
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            GEMINI_REST_ENDPOINT, model_name, self.api_key
        );

        // Create a new client with no timeout for streaming (SSE keeps connection open)
        let streaming_client = Client::builder().build().unwrap_or_else(|_| Client::new());

        let response = streaming_client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| GeminiError::ApiError(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(GeminiError::ApiError(format!(
                "HTTP {status}: {error_text}"
            )));
        }

        // Check content-type to ensure we're getting SSE
        if let Some(content_type) = response.headers().get("content-type") {
            let ct_str = content_type.to_str().unwrap_or("");
            if !ct_str.contains("text/event-stream") && !ct_str.contains("application/json") {
                return Err(GeminiError::ApiError(format!(
                    "Unexpected content-type: {ct_str} (expected text/event-stream)"
                )));
            }
        }

        let stream = GeminiSseStream::new(response.bytes_stream());
        Ok(stream)
    }

    pub async fn list_models(&self, page_size: Option<u32>) -> Result<ListModelsResponse> {
        let url = if let Some(size) = page_size {
            format!(
                "{}/models?key={}&pageSize={size}",
                GEMINI_REST_ENDPOINT, self.api_key
            )
        } else {
            format!("{}/models?key={}", GEMINI_REST_ENDPOINT, self.api_key)
        };

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| GeminiError::ApiError(format!("HTTP request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GeminiError::ApiError(format!(
                "HTTP {status}: {error_text}"
            )));
        }

        let result: ListModelsResponse = response
            .json()
            .await
            .map_err(|e| GeminiError::ApiError(format!("Failed to parse response: {e}")))?;

        Ok(result)
    }
}
