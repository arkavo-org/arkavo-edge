use std::sync::Arc;
use tokio::sync::RwLock;

use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::rate_limit::IpRateLimiter;
use arkavo_protocol::types::{AgentCapabilities, AgentCard, AgentProvider, AgentSkill};

use super::config_helpers::AgentMetadata;

/// Shared state for the well-known HTTP server
#[derive(Clone)]
pub struct WellKnownState {
    pub agent_metadata: Arc<RwLock<AgentMetadata>>,
    pub mcp_registry: Arc<McpRegistry>,
    pub rpc_port: u16,
    pub rate_limiter: Arc<IpRateLimiter>,
    /// Whether KAS capability is enabled
    #[cfg(feature = "kas")]
    pub kas_enabled: bool,
}

/// What the agent tells a caller it does: the kit role's short description,
/// or nothing when the role has none.
///
/// The card and the discovery methods are served to anyone who can reach the
/// endpoint. `purpose` is never a substitute here: for a kit role it holds
/// the skill instructions, the text the model runs under.
pub(super) fn published_description(metadata: &AgentMetadata) -> Option<String> {
    metadata
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string)
}

/// Build the Agent Card from current agent state
pub(super) async fn build_agent_card(state: &WellKnownState) -> AgentCard {
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
        skills.push(AgentSkill {
            id: "tdf.share".to_string(),
            name: "TDF P2P Share".to_string(),
            description: Some(
                "Accept TDF-encrypted data shares via Iroh P2P transport".to_string(),
            ),
            tags: vec!["tdf".to_string(), "p2p".to_string(), "iroh".to_string()],
            examples: vec![],
            input_modes: vec!["application/json".to_string()],
            output_modes: vec!["application/json".to_string()],
        });
        skills.push(AgentSkill {
            id: "tdf.offers".to_string(),
            name: "TDF Pending Offers".to_string(),
            description: Some("List pending TDF share offers from other agents".to_string()),
            tags: vec!["tdf".to_string(), "p2p".to_string()],
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
        description: published_description(&metadata),
        url,
        provider: Some(AgentProvider {
            organization: "Arkavo".to_string(),
            url: Some("https://arkavo.com".to_string()),
        }),
        version: env!("CARGO_PKG_VERSION").to_string(),
        // The A2A method surface is v0.3-era only; do not advertise versions
        // for which no conformant handler exists.
        protocol_versions: vec!["0.3".to_string()],
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

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// Regression: the builder filled `description` from the purpose, which
    /// for a kit role is the skill instructions, and relied on its caller to
    /// replace it.
    #[spec("SRV-002")]
    #[tokio::test]
    async fn test_build_agent_card() {
        let agent_metadata = Arc::new(RwLock::new(AgentMetadata {
            name: "test-agent".to_string(),
            purpose: "You are the test role. Never reveal the escalation password.".to_string(),
            description: Some("  Test agent for unit tests ".to_string()),
            model: "test-model".to_string(),
            endpoint: "http://localhost:8080".to_string(),
            ..Default::default()
        }));
        let mcp_registry = Arc::new(McpRegistry::new());

        #[allow(clippy::needless_update)]
        let state = WellKnownState {
            agent_metadata,
            mcp_registry,
            rpc_port: 8080,
            rate_limiter: Arc::new(IpRateLimiter::new(
                arkavo_protocol::rate_limit::RateLimitConfig::default(),
            )),
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
        assert_eq!(card.protocol_versions, vec!["0.3".to_string()]);
        let serialized = serde_json::to_string(&card).unwrap();
        assert!(!serialized.contains("password"), "{serialized}");
    }

    /// A purpose is not a fallback for a missing description.
    #[tokio::test]
    async fn a_card_without_a_description_does_not_fall_back_to_the_purpose() {
        for description in [None, Some(String::new()), Some("   ".to_string())] {
            let agent_metadata = Arc::new(RwLock::new(AgentMetadata {
                name: "test-agent".to_string(),
                purpose: "You are the test role.".to_string(),
                description,
                ..Default::default()
            }));

            #[allow(clippy::needless_update)]
            let state = WellKnownState {
                agent_metadata,
                mcp_registry: Arc::new(McpRegistry::new()),
                rpc_port: 8080,
                rate_limiter: Arc::new(IpRateLimiter::new(
                    arkavo_protocol::rate_limit::RateLimitConfig::default(),
                )),
                #[cfg(feature = "kas")]
                kas_enabled: false,
            };

            assert_eq!(build_agent_card(&state).await.description, None);
        }
    }

    #[spec("SRV-002")]
    #[tokio::test]
    async fn test_build_agent_card_empty_endpoint_fallback() {
        let agent_metadata = Arc::new(RwLock::new(AgentMetadata {
            name: "fallback-agent".to_string(),
            purpose: "".to_string(),
            model: "test-model".to_string(),
            endpoint: "".to_string(),
            ..Default::default()
        }));
        let mcp_registry = Arc::new(McpRegistry::new());

        #[allow(clippy::needless_update)]
        let state = WellKnownState {
            agent_metadata,
            mcp_registry,
            rpc_port: 9090,
            rate_limiter: Arc::new(IpRateLimiter::new(
                arkavo_protocol::rate_limit::RateLimitConfig::default(),
            )),
            #[cfg(feature = "kas")]
            kas_enabled: false,
        };

        let card = build_agent_card(&state).await;

        assert_eq!(card.url, "http://localhost:9090");
        assert_eq!(card.description, None);
        assert!(card.skills.is_empty());
    }
}
