use anyhow::Result;
use arkavo_dataflow::agent_interface::LlmDataflowAgent;
use serde_json::json;
use warp::http::StatusCode;
use warp::reply::{json, with_status};

pub struct DataflowHandler {
    agent: LlmDataflowAgent,
}

impl DataflowHandler {
    pub fn new() -> Self {
        Self {
            agent: LlmDataflowAgent,
        }
    }

    pub async fn handle_request(
        &self,
        path: Vec<String>,
        body: serde_json::Value,
    ) -> Result<warp::reply::WithStatus<warp::reply::Json>, warp::Rejection> {
        match path.get(0).map(|s| s.as_str()) {
            Some("discover") => Ok(self.discover_capabilities().await),
            Some("configure") => Ok(self.configure_providers(body).await),
            Some("suggest") => Ok(self.suggest_blueprint(body).await),
            _ => Ok(with_status(
                json(&json!({"error": "not_found"})),
                StatusCode::NOT_FOUND,
            )),
        }
    }

    async fn discover_capabilities(&self) -> warp::reply::WithStatus<warp::reply::Json> {
        match self.agent.discover_capabilities().await {
            Ok(capabilities) => with_status(json(&capabilities), StatusCode::OK),
            Err(e) => with_status(
                json(&json!({"error": e.to_string()})),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }

    async fn configure_providers(
        &self,
        body: serde_json::Value,
    ) -> warp::reply::WithStatus<warp::reply::Json> {
        let providers = match serde_json::from_value::<Vec<(String, String)>>(body) {
            Ok(p) => p,
            Err(e) => {
                return with_status(
                    json(&json!({"error": format!("Invalid request body: {}", e)})),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        match self.agent.configure_providers(providers).await {
            Ok(response) => with_status(json(&json!({"message": response})), StatusCode::OK),
            Err(e) => with_status(
                json(&json!({"error": e.to_string()})),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }

    async fn suggest_blueprint(
        &self,
        body: serde_json::Value,
    ) -> warp::reply::WithStatus<warp::reply::Json> {
        let task_description = match body.get("task").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return with_status(
                    json(&json!({"error": "Missing 'task' in request body"})),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        match self
            .agent
            .suggest_blueprint_for_task(task_description)
            .await
        {
            Ok(blueprint) => with_status(json(&blueprint), StatusCode::OK),
            Err(e) => with_status(
                json(&json!({"error": e.to_string()})),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        }
    }
}
