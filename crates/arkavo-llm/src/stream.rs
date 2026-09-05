use crate::provider::InferenceTiming;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct StreamResponse {
    /// Opaque Responses state, populated only on successful terminal chunks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response_items: Vec<serde_json::Value>,
    pub content: String,
    /// Reasoning/thinking content from models with thinking mode (e.g., DeepSeek V3.2-Speciale)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    pub done: bool,
    /// LLM inference timing from local providers (populated on final done=true message)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_timing: Option<InferenceTiming>,
}

impl std::fmt::Debug for StreamResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResponse")
            .field("content", &self.content)
            .field("reasoning_content", &self.reasoning_content)
            .field("done", &self.done)
            .field("inference_timing", &self.inference_timing)
            .field("response_items_count", &self.response_items.len())
            .finish()
    }
}
