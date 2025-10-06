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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: String,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text { text: String },
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
