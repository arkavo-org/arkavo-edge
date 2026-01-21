use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::mcp_registry::McpRegistry;
use crate::types::{AgentCapabilities, AgentCard, AgentProvider, AgentSkill};

use super::config_helpers::AgentMetadata;

/// Shared state for the well-known HTTP server
#[derive(Clone)]
pub struct WellKnownState {
    pub agent_metadata: Arc<RwLock<AgentMetadata>>,
    pub mcp_registry: Arc<McpRegistry>,
    pub rpc_port: u16,
    /// Whether KAS capability is enabled
    #[cfg(feature = "kas")]
    pub kas_enabled: bool,
}

/// Build the Agent Card from current agent state
async fn build_agent_card(state: &WellKnownState) -> AgentCard {
    let metadata = state.agent_metadata.read().await;

    // Get MCP tools to build skills list
    #[allow(unused_mut)]
    let mut skills: Vec<AgentSkill> = match state.mcp_registry.list_all_tools().await {
        Ok(tools) => tools
            .into_iter()
            .map(|t| AgentSkill {
                id: t.name.clone(),
                name: t.name,
                description: Some(t.description),
                tags: vec!["mcp".to_string()],
                examples: vec![],
                input_modes: vec![],
                output_modes: vec![],
            })
            .collect(),
        Err(_) => vec![],
    };

    // Add KAS skills when capability is enabled
    #[cfg(feature = "kas")]
    if state.kas_enabled {
        skills.push(AgentSkill {
            id: "kas.rewrap".to_string(),
            name: "TDF Key Rewrap".to_string(),
            description: Some(
                "Rewrap TDF encryption keys with ABAC policy enforcement".to_string(),
            ),
            tags: vec![
                "kas".to_string(),
                "tdf".to_string(),
                "encryption".to_string(),
            ],
            examples: vec![],
            input_modes: vec!["application/json".to_string()],
            output_modes: vec!["application/json".to_string()],
        });
        skills.push(AgentSkill {
            id: "kas.publicKey".to_string(),
            name: "KAS Public Key".to_string(),
            description: Some("Get KAS public key for TDF encryption".to_string()),
            tags: vec!["kas".to_string(), "crypto".to_string()],
            examples: vec![],
            input_modes: vec!["application/json".to_string()],
            output_modes: vec!["application/json".to_string()],
        });
    }

    // Build capabilities from agent features
    let capabilities = AgentCapabilities {
        streaming: true,
        push_notifications: false,
        state_transition_history: true,
    };

    // Determine the URL based on endpoint or construct from port
    let url = if metadata.endpoint.is_empty() {
        format!("http://localhost:{}", state.rpc_port)
    } else {
        metadata.endpoint.clone()
    };

    AgentCard {
        name: metadata.name.clone(),
        description: if metadata.purpose.is_empty() {
            None
        } else {
            Some(metadata.purpose.clone())
        },
        url,
        provider: Some(AgentProvider {
            organization: "Arkavo".to_string(),
            url: Some("https://arkavo.com".to_string()),
        }),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_versions: vec!["0.3".to_string(), "1.0".to_string()],
        default_input_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        default_output_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        capabilities,
        skills,
        security_schemes: vec![],
        security: vec![],
        extensions: vec![],
        signature: None,
    }
}

/// Handler for GET /.well-known/agent.json
async fn get_agent_json(State(state): State<WellKnownState>) -> impl IntoResponse {
    let agent_card = build_agent_card(&state).await;
    (StatusCode::OK, Json(agent_card))
}

/// Create the axum router for well-known endpoints
fn create_well_known_router(state: WellKnownState) -> Router {
    Router::new()
        .route("/.well-known/agent.json", get(get_agent_json))
        .with_state(state)
}

/// Start the HTTP server for well-known endpoints
pub async fn start_well_known_server(
    bind_addr: SocketAddr,
    state: WellKnownState,
) -> Result<(tokio::task::JoinHandle<()>, u16), Box<dyn std::error::Error + Send + Sync>> {
    let router = create_well_known_router(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let actual_addr = listener.local_addr()?;
    let actual_port = actual_addr.port();

    info!(
        "Well-known HTTP server listening on {} (/.well-known/agent.json)",
        actual_addr
    );

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            warn!("Well-known HTTP server error: {}", e);
        }
    });

    Ok((handle, actual_port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_build_agent_card() {
        let agent_metadata = Arc::new(RwLock::new(AgentMetadata {
            name: "test-agent".to_string(),
            purpose: "Test agent for unit tests".to_string(),
            model: "test-model".to_string(),
            endpoint: "http://localhost:8080".to_string(),
            api_keys: Default::default(),
        }));
        let mcp_registry = Arc::new(McpRegistry::new());

        #[allow(clippy::needless_update)]
        let state = WellKnownState {
            agent_metadata,
            mcp_registry,
            rpc_port: 8080,
            #[cfg(feature = "kas")]
            kas_enabled: false,
        };

        let card = build_agent_card(&state).await;

        assert_eq!(card.name, "test-agent");
        assert_eq!(
            card.description,
            Some("Test agent for unit tests".to_string())
        );
        assert_eq!(card.url, "http://localhost:8080");
        assert!(card.capabilities.streaming);
        assert!(card.protocol_versions.contains(&"0.3".to_string()));
    }
}
