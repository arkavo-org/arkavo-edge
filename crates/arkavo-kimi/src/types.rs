use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Thinking mode configuration for K2.5 series models
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    /// Thinking mode enabled (default for thinking models)
    #[default]
    Enabled,
    /// Thinking mode disabled
    Disabled,
}

/// Thinking configuration for K2.5 series models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub thinking_type: ThinkingMode,
}

impl ThinkingConfig {
    /// Create a new thinking configuration
    pub fn new(enabled: bool) -> Self {
        Self {
            thinking_type: if enabled {
                ThinkingMode::Enabled
            } else {
                ThinkingMode::Disabled
            },
        }
    }

    /// Enable thinking mode
    pub fn enabled() -> Self {
        Self {
            thinking_type: ThinkingMode::Enabled,
        }
    }

    /// Disable thinking mode
    pub fn disabled() -> Self {
        Self {
            thinking_type: ThinkingMode::Disabled,
        }
    }
}

/// Message role in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// Kimi model names
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    // Legacy V1 models (Moonshot AI API)
    #[serde(rename = "moonshot-v1-8k")]
    MoonshotV1_8k,
    #[serde(rename = "moonshot-v1-32k")]
    MoonshotV1_32k,
    #[serde(rename = "moonshot-v1-128k")]
    MoonshotV1_128k,
    // K2.5 series models (256K context, Moonshot AI API)
    #[serde(rename = "kimi-k2.5")]
    KimiK2_5,
    #[serde(rename = "kimi-k2-0905-preview")]
    KimiK20905Preview,
    #[serde(rename = "kimi-k2-turbo-preview")]
    KimiK2TurboPreview,
    #[serde(rename = "kimi-k2-thinking")]
    KimiK2Thinking,
    #[serde(rename = "kimi-k2-thinking-turbo")]
    KimiK2ThinkingTurbo,
}

impl Model {
    pub fn as_str(&self) -> &'static str {
        match self {
            Model::MoonshotV1_8k => "moonshot-v1-8k",
            Model::MoonshotV1_32k => "moonshot-v1-32k",
            Model::MoonshotV1_128k => "moonshot-v1-128k",
            Model::KimiK2_5 => "kimi-k2.5",
            Model::KimiK20905Preview => "kimi-k2-0905-preview",
            Model::KimiK2TurboPreview => "kimi-k2-turbo-preview",
            Model::KimiK2Thinking => "kimi-k2-thinking",
            Model::KimiK2ThinkingTurbo => "kimi-k2-thinking-turbo",
        }
    }

    /// Get the context length for this model
    pub fn context_length(&self) -> u32 {
        match self {
            Model::MoonshotV1_8k => 8_192,
            Model::MoonshotV1_32k => 32_768,
            Model::MoonshotV1_128k => 131_072,
            // K2.5 series all have 256K context
            Model::KimiK2_5
            | Model::KimiK20905Preview
            | Model::KimiK2TurboPreview
            | Model::KimiK2Thinking
            | Model::KimiK2ThinkingTurbo => 262_144,
        }
    }

    /// Check if this model supports thinking mode
    pub fn supports_thinking(&self) -> bool {
        matches!(
            self,
            Model::KimiK2_5
                | Model::KimiK20905Preview
                | Model::KimiK2TurboPreview
                | Model::KimiK2Thinking
                | Model::KimiK2ThinkingTurbo
        )
    }

    /// Check if this is a legacy V1 model
    pub fn is_legacy_v1(&self) -> bool {
        matches!(
            self,
            Model::MoonshotV1_8k | Model::MoonshotV1_32k | Model::MoonshotV1_128k
        )
    }
}

/// Chat message for Kimi API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Tool/function definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String, // "function"
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Tool call in a response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Tool choice options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "none")]
    None,
    Required(ToolChoiceRequired),
    Function(ToolChoiceFunction),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceRequired {
    #[serde(rename = "type")]
    pub choice_type: String, // "required"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    #[serde(rename = "type")]
    pub choice_type: String, // "function"
    pub function: ToolChoiceFunctionName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceFunctionName {
    pub name: String,
}

/// Chat completion request
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    /// Thinking mode configuration (K2.5 series only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
}

/// Chat completion response
#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
    /// Reasoning content from thinking mode (K2.5 series)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Streaming response chunk
#[derive(Debug, Deserialize)]
pub struct StreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Delta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
    /// Reasoning content from thinking mode (K2.5 series)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeltaToolCall {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub tool_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<DeltaFunction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeltaFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Error response
#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
    pub code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_role_serialization() {
        // Test serialization
        assert_eq!(
            serde_json::to_string(&ChatRole::System).unwrap(),
            "\"system\""
        );
        assert_eq!(serde_json::to_string(&ChatRole::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&ChatRole::Assistant).unwrap(),
            "\"assistant\""
        );

        // Test deserialization
        assert_eq!(
            serde_json::from_str::<ChatRole>("\"system\"").unwrap(),
            ChatRole::System
        );
        assert_eq!(
            serde_json::from_str::<ChatRole>("\"user\"").unwrap(),
            ChatRole::User
        );
        assert_eq!(
            serde_json::from_str::<ChatRole>("\"assistant\"").unwrap(),
            ChatRole::Assistant
        );
    }

    #[test]
    fn test_chat_message_serialization() {
        let msg = ChatMessage {
            role: ChatRole::User,
            content: "Hello".to_string(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"Hello\""));

        // Test round-trip
        let deserialized: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.role, ChatRole::User);
        assert_eq!(deserialized.content, "Hello");
    }

    // K2.5 Model Tests
    #[test]
    fn test_k2_5_model_serialization() {
        // Test K2.5 model serialization
        assert_eq!(
            serde_json::to_string(&Model::KimiK2_5).unwrap(),
            "\"kimi-k2.5\""
        );
        assert_eq!(
            serde_json::to_string(&Model::KimiK20905Preview).unwrap(),
            "\"kimi-k2-0905-preview\""
        );
        assert_eq!(
            serde_json::to_string(&Model::KimiK2TurboPreview).unwrap(),
            "\"kimi-k2-turbo-preview\""
        );
        assert_eq!(
            serde_json::to_string(&Model::KimiK2Thinking).unwrap(),
            "\"kimi-k2-thinking\""
        );
        assert_eq!(
            serde_json::to_string(&Model::KimiK2ThinkingTurbo).unwrap(),
            "\"kimi-k2-thinking-turbo\""
        );
    }

    #[test]
    fn test_model_as_str() {
        assert_eq!(Model::MoonshotV1_8k.as_str(), "moonshot-v1-8k");
        assert_eq!(Model::MoonshotV1_32k.as_str(), "moonshot-v1-32k");
        assert_eq!(Model::MoonshotV1_128k.as_str(), "moonshot-v1-128k");
        assert_eq!(Model::KimiK2_5.as_str(), "kimi-k2.5");
        assert_eq!(Model::KimiK20905Preview.as_str(), "kimi-k2-0905-preview");
        assert_eq!(Model::KimiK2TurboPreview.as_str(), "kimi-k2-turbo-preview");
        assert_eq!(Model::KimiK2Thinking.as_str(), "kimi-k2-thinking");
        assert_eq!(
            Model::KimiK2ThinkingTurbo.as_str(),
            "kimi-k2-thinking-turbo"
        );
    }

    #[test]
    fn test_model_context_length() {
        // Legacy V1 models
        assert_eq!(Model::MoonshotV1_8k.context_length(), 8_192);
        assert_eq!(Model::MoonshotV1_32k.context_length(), 32_768);
        assert_eq!(Model::MoonshotV1_128k.context_length(), 131_072);

        // K2.5 series - all have 256K context
        assert_eq!(Model::KimiK2_5.context_length(), 262_144);
        assert_eq!(Model::KimiK20905Preview.context_length(), 262_144);
        assert_eq!(Model::KimiK2TurboPreview.context_length(), 262_144);
        assert_eq!(Model::KimiK2Thinking.context_length(), 262_144);
        assert_eq!(Model::KimiK2ThinkingTurbo.context_length(), 262_144);
    }

    #[test]
    fn test_model_supports_thinking() {
        // Legacy models don't support thinking
        assert!(!Model::MoonshotV1_8k.supports_thinking());
        assert!(!Model::MoonshotV1_32k.supports_thinking());
        assert!(!Model::MoonshotV1_128k.supports_thinking());

        // K2.5 series supports thinking
        assert!(Model::KimiK2_5.supports_thinking());
        assert!(Model::KimiK20905Preview.supports_thinking());
        assert!(Model::KimiK2TurboPreview.supports_thinking());
        assert!(Model::KimiK2Thinking.supports_thinking());
        assert!(Model::KimiK2ThinkingTurbo.supports_thinking());
    }

    #[test]
    fn test_model_is_legacy_v1() {
        assert!(Model::MoonshotV1_8k.is_legacy_v1());
        assert!(Model::MoonshotV1_32k.is_legacy_v1());
        assert!(Model::MoonshotV1_128k.is_legacy_v1());

        assert!(!Model::KimiK2_5.is_legacy_v1());
        assert!(!Model::KimiK20905Preview.is_legacy_v1());
        assert!(!Model::KimiK2TurboPreview.is_legacy_v1());
        assert!(!Model::KimiK2Thinking.is_legacy_v1());
        assert!(!Model::KimiK2ThinkingTurbo.is_legacy_v1());
    }

    // Thinking Mode Tests
    #[test]
    fn test_thinking_config_serialization() {
        let enabled = ThinkingConfig::enabled();
        let json = serde_json::to_string(&enabled).unwrap();
        assert!(json.contains("enabled"));

        let disabled = ThinkingConfig::disabled();
        let json = serde_json::to_string(&disabled).unwrap();
        assert!(json.contains("disabled"));
    }

    #[test]
    fn test_thinking_config_new() {
        let enabled = ThinkingConfig::new(true);
        assert!(matches!(enabled.thinking_type, ThinkingMode::Enabled));

        let disabled = ThinkingConfig::new(false);
        assert!(matches!(disabled.thinking_type, ThinkingMode::Disabled));
    }

    #[test]
    fn test_thinking_mode_default() {
        let default: ThinkingMode = Default::default();
        assert!(matches!(default, ThinkingMode::Enabled));
    }

    #[test]
    fn test_chat_completion_request_with_thinking() {
        let request = ChatCompletionRequest {
            model: "kimi-k2.5".to_string(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "Hello".to_string(),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: None,
            tool_choice: None,
            temperature: Some(0.7),
            top_p: None,
            max_tokens: None,
            stream: Some(false),
            presence_penalty: None,
            frequency_penalty: None,
            n: None,
            stop: None,
            thinking: Some(ThinkingConfig::enabled()),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"thinking\""));
        assert!(json.contains("enabled"));
        assert!(json.contains("kimi-k2.5"));
    }
}
