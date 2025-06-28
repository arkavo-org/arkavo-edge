use crate::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(super) struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatResponse {
    pub message: Message,
    pub done: bool,
}
