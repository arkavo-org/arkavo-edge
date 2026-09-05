use crate::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(super) struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
}

impl ChatRequest {
    pub(super) fn new(model: String, mut messages: Vec<Message>, stream: bool) -> Self {
        // Provider-owned encrypted state must not cross into another provider.
        for message in &mut messages {
            message.response_items.clear();
        }
        Self {
            model,
            messages,
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
        message.response_items =
            vec![serde_json::json!({"type":"reasoning", "encrypted_content":"opaque-canary"})];
        let request = ChatRequest::new("local-model".into(), vec![message], false);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("visible answer"));
        assert!(!json.contains("response_items"));
        assert!(!json.contains("opaque-canary"));
    }
}
