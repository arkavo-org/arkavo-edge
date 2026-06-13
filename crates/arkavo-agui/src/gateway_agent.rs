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
