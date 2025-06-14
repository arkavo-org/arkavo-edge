use crate::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub stream: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub message: Message,
    pub done: bool,
}
