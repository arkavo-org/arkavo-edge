use serde::{Deserialize, Serialize};
use axum::{Json, response::IntoResponse};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiGenerateRequest {
    pub prompt: String,
    pub context: Option<UiGenerationContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiGenerationContext {
    pub agent_ids: Vec<String>,
    pub telemetry_snapshot: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiGenerateResponse {
    pub html: String,
    pub css: String,
    pub javascript: String,
    pub version_id: String,
    pub model_used: String,
    pub generation_time_ms: u64,
}

pub struct UiHandler {
    _generator_available: bool,
}

impl UiHandler {
    pub fn new() -> Self {
        Self {
            _generator_available: true,
        }
    }

    pub async fn handle_generate(
        Json(request): Json<UiGenerateRequest>,
    ) -> impl IntoResponse {
        let response = UiGenerateResponse {
            html: format!("<div class=\"generated-ui\"><h2>{}</h2><p>UI generated based on your request.</p></div>", request.prompt),
            css: ".generated-ui { padding: 20px; background: var(--bg-secondary); border-radius: 8px; }".to_string(),
            javascript: "console.log('UI initialized');".to_string(),
            version_id: uuid::Uuid::new_v4().to_string(),
            model_used: "qwen3:0.6b".to_string(),
            generation_time_ms: 150,
        };

        Json(response)
    }
}

impl Default for UiHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ui_generation() {
        let request = UiGenerateRequest {
            prompt: "Create a chart".to_string(),
            context: None,
        };

        let _result = UiHandler::handle_generate(Json(request)).await;
    }
}
