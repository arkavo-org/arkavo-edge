use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "functionDeclarations")]
    pub function_declarations: Vec<FunctionDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupConfig {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "generationConfig")]
    pub generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    #[serde(rename = "responseModalities")]
    pub response_modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxOutputTokens")]
    pub max_output_tokens: Option<u32>,
}

impl SetupConfig {
    pub fn new_text_only(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            generation_config: Some(GenerationConfig {
                response_modalities: vec!["TEXT".to_string()],
                temperature: None,
                max_output_tokens: None,
            }),
            tools: None,
        }
    }

    pub fn new_audio(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            generation_config: Some(GenerationConfig {
                response_modalities: vec!["AUDIO".to_string()],
                temperature: None,
                max_output_tokens: None,
            }),
            tools: None,
        }
    }

    pub fn new_multimodal(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            generation_config: Some(GenerationConfig {
                response_modalities: vec!["TEXT".to_string()],
                temperature: Some(0.7),
                max_output_tokens: Some(4096),
            }),
            tools: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientContent {
    pub turns: Vec<Turn>,
    #[serde(rename = "turnComplete")]
    pub turn_complete: bool,
}

impl ClientContent {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            turns: vec![Turn {
                role: "USER".to_string(),
                parts: vec![Part::Text { text: text.into() }],
            }],
            turn_complete: true,
        }
    }

    pub fn from_text_and_image(
        text: impl Into<String>,
        image_base64: String,
        mime_type: String,
    ) -> Self {
        Self {
            turns: vec![Turn {
                role: "USER".to_string(),
                parts: vec![
                    Part::Text { text: text.into() },
                    Part::InlineData {
                        inline_data: InlineData {
                            mime_type,
                            data: image_base64,
                        },
                    },
                ],
            }],
            turn_complete: true,
        }
    }

    pub fn from_image(image_base64: String, mime_type: String) -> Self {
        Self {
            turns: vec![Turn {
                role: "USER".to_string(),
                parts: vec![Part::InlineData {
                    inline_data: InlineData {
                        mime_type,
                        data: image_base64,
                    },
                }],
            }],
            turn_complete: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: String,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub function_calls: Vec<FunctionCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Value,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResponse {
    pub function_responses: Vec<FunctionResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
    pub id: String,
    pub response: Value,
}

impl ToolResponse {
    pub fn new(id: impl Into<String>, response: Value) -> Self {
        Self {
            function_responses: vec![FunctionResponse {
                id: id.into(),
                response,
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerMessage {
    SetupComplete {},
    #[serde(rename = "toolCall")]
    ToolCall {
        #[serde(rename = "toolCall")]
        tool_call: ToolCall,
    },
    #[serde(rename = "serverContent")]
    ServerContent {
        #[serde(rename = "serverContent")]
        server_content: ServerContent,
    },
    #[serde(rename = "goAway")]
    GoAway {
        #[serde(rename = "goAway")]
        go_away: GoAway,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerContent {
    pub model_turn: Option<ModelTurn>,
    #[serde(rename = "turnComplete")]
    pub turn_complete: bool,
}

impl ServerContent {
    pub fn extract_text(&self) -> Option<String> {
        self.model_turn
            .as_ref()?
            .parts
            .iter()
            .find_map(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            })
    }

    pub fn extract_all_text(&self) -> String {
        self.model_turn
            .as_ref()
            .map(|turn| {
                turn.parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTurn {
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoAway {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClientMessage {
    Setup {
        setup: SetupConfig,
    },
    ClientContent {
        #[serde(rename = "clientContent")]
        client_content: ClientContent,
    },
    ToolResponse {
        #[serde(rename = "toolResponse")]
        tool_response: ToolResponse,
    },
}

// Streaming API types
#[derive(Debug, Clone, Deserialize)]
pub struct StreamChunk {
    #[serde(default)]
    pub candidates: Vec<StreamCandidate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamCandidate {
    pub content: StreamContent,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamContent {
    #[serde(default)]
    pub parts: Vec<StreamPart>,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StreamPart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: StreamFunctionCall,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamFunctionCall {
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone)]
pub struct StreamResponse {
    pub text: Option<String>,
    pub function_calls: Vec<FunctionCall>,
    pub done: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub name: String,
    #[serde(default)]
    pub base_model_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_token_limit: Option<u32>,
    #[serde(default)]
    pub output_token_limit: Option<u32>,
    #[serde(default)]
    pub supported_generation_methods: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListModelsResponse {
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}
