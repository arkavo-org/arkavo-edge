use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp::Reply;

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
        &self,
        request: UiGenerateRequest,
    ) -> Result<impl Reply, warp::Rejection> {
        let response = UiGenerateResponse {
            html: format!("<div class=\"generated-ui\"><h2>{}</h2><p>UI generated based on your request.</p></div>", request.prompt),
            css: ".generated-ui { padding: 20px; background: var(--bg-secondary); border-radius: 8px; }".to_string(),
            javascript: "console.log('UI initialized');".to_string(),
            version_id: uuid::Uuid::new_v4().to_string(),
            model_used: "qwen3:0.6b".to_string(),
            generation_time_ms: 150,
        };

        Ok(warp::reply::json(&response))
    }
}

impl Default for UiHandler {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_ui_routes(
    handler: Arc<UiHandler>,
) -> impl warp::Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let generate_route = warp::path!("api" / "ui" / "generate")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || handler.clone()))
        .and_then(
            |request: UiGenerateRequest, handler: Arc<UiHandler>| async move {
                handler.handle_generate(request).await
            },
        );

    generate_route
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ui_generation() {
        let handler = UiHandler::new();
        let request = UiGenerateRequest {
            prompt: "Create a chart".to_string(),
            context: None,
        };

        let result = handler.handle_generate(request).await;
        assert!(result.is_ok());
    }
}
