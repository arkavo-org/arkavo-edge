//! Spec-compliant HTTP/SSE `/api/agent` run endpoint.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use arkavo_agui_protocol::{AgentCapabilities, RunAgentInput};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::Event, sse::Sse},
};
use futures::StreamExt;

use crate::agent_connection::AgentConnection;
use crate::gateway::AppState;

#[derive(Debug, serde::Deserialize)]
pub struct RunAgentParams {
    #[serde(rename = "agentId")]
    agent_id: Option<String>,
}

pub async fn run_agent_handler(
    Query(params): Query<RunAgentParams>,
    State(state): State<AppState>,
    Json(input): Json<RunAgentInput>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, Response> {
    let agent_id = resolve_agent(&state.agent_connections, params.agent_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "No agent available for the requested run",
            )
                .into_response()
        })?;

    let connection = {
        let conns = state.agent_connections.read().await;
        conns.get(&agent_id).cloned()
    }
    .ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent disconnected before run could start",
        )
            .into_response()
    })?;

    let event_stream = crate::agui_event_stream::run_event_stream(connection, input);
    let sse_stream = async_stream::stream! {
        let mut event_stream = std::pin::pin!(event_stream);
        while let Some(event) = event_stream.next().await {
            yield Ok::<_, Infallible>(
                Event::default().json_data(event).unwrap_or_else(|_| {
                    Event::default().event("error").data("failed to serialize event")
                }),
            );
        }
    };

    Ok(Sse::new(sse_stream).keep_alive(axum::response::sse::KeepAlive::new()))
}

pub async fn capabilities_handler() -> Json<AgentCapabilities> {
    Json(AgentCapabilities::arkavo_default())
}

async fn resolve_agent(
    agent_connections: &Arc<tokio::sync::RwLock<HashMap<String, Arc<AgentConnection>>>>,
    agent_id: Option<String>,
) -> Option<String> {
    if let Some(id) = agent_id {
        let conns = agent_connections.read().await;
        if conns.contains_key(&id) {
            return Some(id);
        }
    }
    let conns = agent_connections.read().await;
    conns.keys().next().cloned()
}
#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};

    use arkavo_agui_protocol::RunAgentInput;
    use axum::extract::{Query, State};

    use super::*;
    use crate::gateway::AppState;

    fn test_app_state() -> AppState {
        AppState {
            connections: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            agents: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            agent_connections: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            budget_handler: Arc::new(tokio::sync::RwLock::new(
                crate::budget_handler::BudgetHandler::new(),
            )),
            cost_handler: Arc::new(tokio::sync::RwLock::new(
                crate::cost_handler::CostHandler::new(),
            )),
            security_handler: Arc::new(tokio::sync::RwLock::new(
                crate::security_handler::SecurityHandler::new(),
            )),
            initial_prompt: Arc::new(tokio::sync::RwLock::new(None)),
            dataflow_handler: Arc::new(crate::dataflow_handler::DataflowHandler::new()),
            debug_handler: None,
            rate_limiter: Arc::new(arkavo_protocol::rate_limit::IpRateLimiter::new(
                arkavo_protocol::rate_limit::RateLimitConfig::default(),
            )),
            task_store: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            learning_module: Arc::new(tokio::sync::RwLock::new(
                arkavo_router::learning::LearningModule::new(),
            )),
            routing_history: Arc::new(tokio::sync::RwLock::new(VecDeque::new())),
            lesson_tx: None,
            lesson_store: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            context_topology_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            arp_handler: Arc::new(crate::arp_handler::ArpHandler::default()),
            swarm_flights: Arc::new(crate::swarm_flight_registry::SwarmFlightRegistry::new()),
        }
    }

    #[tokio::test]
    async fn capabilities_handler_returns_default_capabilities() {
        let Json(caps) = capabilities_handler().await;
        assert!(caps.transport.sse);
        assert!(caps.tools.enabled);
    }

    #[tokio::test]
    async fn run_agent_handler_returns_503_when_no_agent_available() {
        let state = test_app_state();
        let params = Query(RunAgentParams { agent_id: None });
        let input = RunAgentInput::new("thread-1", "run-1");
        let result = run_agent_handler(params, State(state), Json(input)).await;
        assert!(result.is_err());
        let response = result.unwrap_err();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
