use crate::provider::InferenceTiming;
use crate::provider_state::ProviderState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamResponse {
    /// Opaque provider state, populated only on successful terminal chunks.
    #[serde(default, skip_serializing_if = "ProviderState::is_empty")]
    pub provider_state: ProviderState,
    pub content: String,
    /// Reasoning/thinking content from models with thinking mode (e.g., DeepSeek V3.2-Speciale)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    pub done: bool,
    /// LLM inference timing from local providers (populated on final done=true message)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_timing: Option<InferenceTiming>,
}
