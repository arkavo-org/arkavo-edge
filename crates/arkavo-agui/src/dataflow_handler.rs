use arkavo_dataflow::agent_interface::LlmDataflowAgent;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub struct DataflowHandler {
    agent: LlmDataflowAgent,
}

impl Default for DataflowHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl DataflowHandler {
    pub fn new() -> Self {
        Self {
            agent: LlmDataflowAgent,
        }
    }

    pub async fn handle_request(&self, path: Vec<String>, body: serde_json::Value) -> Response {
        match path.first().map(|s| s.as_str()) {
            Some("discover") => self.discover_capabilities().await,
            Some("configure") => self.configure_providers(body).await,
            Some("suggest") => self.suggest_blueprint(body).await,
            _ => (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response(),
        }
    }

    async fn discover_capabilities(&self) -> Response {
        match self.agent.discover_capabilities().await {
            Ok(capabilities) => (StatusCode::OK, Json(capabilities)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    }

    async fn configure_providers(&self, body: serde_json::Value) -> Response {
        let providers = match serde_json::from_value::<Vec<(String, String)>>(body) {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("Invalid request body: {}", e)})),
                )
                    .into_response();
            }
        };

        match self.agent.configure_providers(providers).await {
            Ok(response) => (StatusCode::OK, Json(json!({"message": response}))).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    }

    async fn suggest_blueprint(&self, body: serde_json::Value) -> Response {
        let task_description = match body.get("task").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "Missing 'task' in request body"})),
                )
                    .into_response();
            }
        };

        match self
            .agent
            .suggest_blueprint_for_task(task_description)
            .await
        {
            Ok(blueprint) => (StatusCode::OK, Json(blueprint)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    }
}
