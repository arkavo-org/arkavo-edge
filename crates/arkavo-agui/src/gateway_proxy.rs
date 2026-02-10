use axum::{Json, extract::State, response::Response};
use std::sync::Arc;
use tokio::sync::RwLock;

pub async fn agent_proxy_handler(
    axum::extract::Path(agent_id): axum::extract::Path<String>,
    State(state): State<super::gateway::AppState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    handle_agent_proxy(agent_id, body, state.agents).await
}

pub async fn dataflow_handler(
    State(state): State<super::gateway::AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let path_vec = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect::<Vec<String>>();

    state.dataflow_handler.handle_request(path_vec, body).await
}

async fn handle_agent_proxy(
    agent_id: String,
    body: serde_json::Value,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
) -> Json<serde_json::Value> {
    let agents_list = agents.read().await;
    let agent = agents_list
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(&agent_id));

    if let Some(agent_info) = agent {
        if let Some(endpoint) = agent_info.get("endpoint").and_then(|v| v.as_str()) {
            let request_id = body.get("id").cloned();
            match forward_to_agent(endpoint, body).await {
                Ok(response) => Json(response),
                Err(e) => {
                    let error_response = serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": {
                            "code": -32603,
                            "message": format!("Failed to forward request: {}", e)
                        },
                        "id": request_id
                    });
                    Json(error_response)
                }
            }
        } else {
            let error_response = serde_json::json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32602,
                    "message": "Agent endpoint not found"
                },
                "id": body.get("id").cloned()
            });
            Json(error_response)
        }
    } else {
        let error_response = serde_json::json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -32602,
                "message": format!("Agent {agent_id} not found")
            },
            "id": body.get("id").cloned()
        });
        Json(error_response)
    }
}

async fn forward_to_agent(
    endpoint: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://{endpoint}");

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let json_response = response.json::<serde_json::Value>().await?;
    Ok(json_response)
}
