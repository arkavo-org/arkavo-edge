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

#[derive(Debug, Clone, Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxOutputTokens")]
    max_output_tokens: Option<u32>,
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
            .timeout(Duration::from_secs(30))
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
    ) -> Result<(Option<String>, Vec<FunctionCall>)> {
        let request = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![Part::Text {
                    text: prompt.into(),
                }],
            }],
            tools: tools.map(|t| {
                vec![Tool {
                    function_declarations: t,
                }]
            }),
            generation_config: None,
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

    pub async fn stream_generate_content(
        &self,
        prompt: impl Into<String>,
        tools: Option<Vec<FunctionDeclaration>>,
    ) -> Result<GeminiSseStream> {
        let request = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![Part::Text {
                    text: prompt.into(),
                }],
            }],
            tools: tools.map(|t| {
                vec![Tool {
                    function_declarations: t,
                }]
            }),
            generation_config: None,
        };

        let model_name = self.model.strip_prefix("models/").unwrap_or(&self.model);
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            GEMINI_REST_ENDPOINT, model_name, self.api_key
        );

        let response = self
            .client
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

        Ok(GeminiSseStream::new(response.bytes_stream()))
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
