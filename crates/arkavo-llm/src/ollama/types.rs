use crate::Message;
use crate::message::{Role, ToolCall};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(super) struct ChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    pub stream: bool,
}

/// Ollama's wire shape for one chat message.
///
/// It restates every field of the neutral `Message` except the provider's
/// opaque state, for which it has no slot at all. Serializing `Message`
/// directly would put another provider's encrypted items on this wire the
/// moment a strip step is forgotten; a type that cannot hold them removes the
/// possibility rather than guarding against it. `wire_message_matches_the_neutral_message_shape`
/// keeps the restated fields from drifting apart.
#[derive(Debug, Serialize)]
pub(super) struct OllamaMessage {
    role: Role,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ToolCall>,
}

impl From<Message> for OllamaMessage {
    fn from(message: Message) -> Self {
        Self {
            role: message.role,
            content: message.content,
            images: message.images,
            tool_call_id: message.tool_call_id,
            tool_name: message.tool_name,
            tool_calls: message.tool_calls,
        }
    }
}

impl ChatRequest {
    pub(super) fn new(model: String, messages: Vec<Message>, stream: bool) -> Self {
        Self {
            model,
            messages: messages.into_iter().map(OllamaMessage::from).collect(),
            stream,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatResponse {
    pub message: Message,
    pub done: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[arkavo_test_macros::spec("ASTRA-002")]
    #[test]
    fn switching_to_ollama_does_not_forward_openai_state() {
        let mut message = Message::assistant("visible answer");
        message.provider_state = crate::ProviderState::openai_responses(vec![
            serde_json::json!({"type":"reasoning", "encrypted_content":"opaque-canary"}),
        ]);
        let request = ChatRequest::new("local-model".into(), vec![message], false);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("visible answer"));
        assert!(!json.contains("provider_state"));
        assert!(!json.contains("opaque-canary"));
    }

    /// The wire type is a hand-restated copy of `Message` minus provider state,
    /// so a field added to `Message` that Ollama should send must be added here
    /// too. Comparing the two serializations is what makes that visible.
    #[arkavo_test_macros::spec("ASTRA-002")]
    #[test]
    fn wire_message_matches_the_neutral_message_shape() {
        let mut message = Message::tool_result("12:00 UTC", "call_1", "get_time");
        message.images = Some(vec!["base64image".into()]);
        message.tool_calls = vec![ToolCall {
            name: "get_time".into(),
            arguments: "{}".into(),
            id: Some("call_1".into()),
        }];
        let neutral = serde_json::to_value(&message).unwrap();
        let wire = serde_json::to_value(OllamaMessage::from(message)).unwrap();
        assert_eq!(neutral, wire);
    }
}
