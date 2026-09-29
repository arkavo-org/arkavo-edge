use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::openrpc;
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    AgentDiscoverFilter, DiscoverFeaturesDisclose, DiscoverFeaturesQuery, DiscoveredAgent,
    FeatureDisclosure, FeatureType,
};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;

use super::super::config_helpers::AgentMetadata;
use super::super::well_known::published_description;

#[allow(clippy::too_many_arguments)]
pub async fn handle_agent_discover(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    mcp_registry: &Arc<McpRegistry>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    event_writer: Option<&Arc<EventWriter>>,
    session_id: &str,
    event_sequence: &Arc<tokio::sync::RwLock<u64>>,
    filter: Option<AgentDiscoverFilter>,
) -> Result<Vec<DiscoveredAgent>, ErrorObjectOwned> {
    let timer = RpcTimer::new("agent_discover".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    // Emit agent discover event
    if let Some(writer) = event_writer {
        let agent_meta = agent_metadata.read().await;
        let mut seq = event_sequence.write().await;
        let sequence = *seq;
        *seq += 1;

        let event = Event::new(
            session_id.to_string(),
            sequence,
            agent_meta.name.clone(),
            EventPayload::ToolCall {
                tool_name: "agent_discover".to_string(),
                parameters: serde_json::json!({"filter": filter}),
                tool_call_id: Some(uuid::Uuid::new_v4().to_string()),
            },
        );
        let _ = writer.write(event).await;
    }

    // Get MCP tools and server status
    let mcp_tools = match mcp_registry.list_all_tools().await {
        Ok(tools) => tools.into_iter().map(|t| t.name).collect::<Vec<String>>(),
        Err(_) => Vec::new(),
    };

    let mcp_servers = mcp_registry.get_server_status().await;

    // Build metadata with MCP information. `purpose` keeps its name for the
    // callers that read it and carries the role's short description: this
    // method answers anyone who can reach the endpoint, and the agent's own
    // purpose is the text the model runs under.
    let (name, purpose, model, endpoint) = {
        let metadata = agent_metadata.read().await;
        (
            metadata.name.clone(),
            published_description(&metadata).unwrap_or_default(),
            metadata.model.clone(),
            metadata.endpoint.clone(),
        )
    };

    let metadata_json = serde_json::json!({
        "name": name,
        "purpose": purpose,
        "model": model,
        "mcp_tools": mcp_tools,
        "mcp_servers": mcp_servers,
    });

    let task_types = vec![
        "chat".to_string(),
        "code_editing".to_string(),
        "test_execution".to_string(),
        "diff_preview".to_string(),
    ];

    let metadata_with_tasks = {
        let mut meta = metadata_json;
        meta["task_capabilities"] = serde_json::json!([
            {
                "type": "chat",
                "constraints": {
                    "max_context_length": 8192,
                    "supports_streaming": true,
                }
            },
            {
                "type": "code_editing",
                "constraints": {
                    "languages": ["rust", "python", "javascript", "typescript"],
                    "supports_refactoring": true,
                }
            },
            {
                "type": "test_execution",
                "constraints": {
                    "frameworks": ["cargo", "pytest", "jest"],
                }
            },
            {
                "type": "diff_preview",
                "constraints": {
                    "formats": ["unified", "split"],
                }
            }
        ]);
        meta
    };

    let agent = DiscoveredAgent {
        agent_id: uuid::Uuid::new_v4(),
        endpoint,
        tasks: Some(task_types),
        metadata: Some(metadata_with_tasks),
    };

    timer.success();
    Ok(vec![agent])
}

pub async fn handle_discover_features_query(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    mcp_registry: &Arc<McpRegistry>,
    query: Option<DiscoverFeaturesQuery>,
) -> Result<DiscoverFeaturesDisclose, ErrorObjectOwned> {
    let timer = RpcTimer::new("discover_features_query".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let mut disclosures = Vec::new();

    // Add base protocol support
    disclosures.push(FeatureDisclosure {
        feature_type: FeatureType::Protocol,
        id: "https://didcomm.org/discover-features/2.0".to_string(),
        roles: Some(vec!["requester".to_string(), "responder".to_string()]),
    });

    // Add A2A protocol support
    disclosures.push(FeatureDisclosure {
        feature_type: FeatureType::Protocol,
        id: "https://arkavo.org/a2a/1.0".to_string(),
        roles: Some(vec!["agent".to_string()]),
    });

    // Add MCP tools if available
    if let Ok(tools) = mcp_registry.list_all_tools().await {
        for tool in tools {
            disclosures.push(FeatureDisclosure {
                feature_type: FeatureType::McpTool,
                id: tool.name,
                roles: None,
            });
        }
    }

    // Add MCP servers
    let mcp_servers = mcp_registry.get_server_status().await;
    for (server_name, status) in mcp_servers {
        disclosures.push(FeatureDisclosure {
            feature_type: FeatureType::McpServer,
            id: format!("{server_name} ({status})"),
            roles: None,
        });
    }

    // Filter based on query if provided
    if let Some(query) = query
        && let Some(queries) = query.queries
    {
        disclosures.retain(|disclosure| {
            queries.iter().any(|q| {
                if q.feature_type as i32 != disclosure.feature_type as i32 {
                    return false;
                }
                if let Some(pattern) = &q.match_pattern {
                    if pattern.contains('*') {
                        let prefix = pattern.trim_end_matches('*');
                        disclosure.id.starts_with(prefix)
                    } else {
                        disclosure.id == *pattern
                    }
                } else {
                    true
                }
            })
        });
    }

    timer.success();
    Ok(DiscoverFeaturesDisclose { disclosures })
}

#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
pub async fn handle_rpc_discover(
    metrics: &Arc<MetricsCollector>,
) -> Result<serde_json::Value, ErrorObjectOwned> {
    let timer = RpcTimer::new("rpc.discover".to_string(), metrics.clone());
    let schema = openrpc::generate_openrpc_schema();

    match serde_json::to_value(schema) {
        Ok(value) => {
            timer.success();
            Ok(value)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32603,
                "Failed to serialize OpenRPC schema",
                Some(e.to_string()),
            ))
        }
    }
}

/// Handle agent.capabilities.get RPC method
/// Returns comprehensive agent metadata for orchestrator onboarding
#[allow(clippy::too_many_arguments)]
pub async fn handle_agent_capabilities_get(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    mcp_registry: &Arc<McpRegistry>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    public_key: Option<&str>,
) -> Result<arkavo_protocol::types::AgentCapabilitiesGetResponse, ErrorObjectOwned> {
    use arkavo_protocol::types::{AgentCapabilitiesGetResponse, InteractionMode, McpToolInfo};

    let timer = RpcTimer::new("agent.capabilities.get".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    // Capability tags are coarse routing labels: they are derived from the
    // purpose here, on the agent, and carry none of its text. The `purpose`
    // field of the response is the role's short description, because the
    // agent's own purpose is the text the model runs under.
    let metadata = agent_metadata.read().await;
    let agent_id = metadata.name.clone();
    let name = metadata.name.clone();
    let capabilities = infer_capabilities_from_purpose(&name, &metadata.purpose);
    let purpose = published_description(&metadata).unwrap_or_default();
    let model = metadata.model.clone();
    drop(metadata);

    // Get MCP tools with details
    let mcp_tools = match mcp_registry.list_all_tools().await {
        Ok(tools) => tools
            .into_iter()
            .map(|t| McpToolInfo {
                name: t.name,
                description: t.description,
                server: None,
                input_schema: t.input_schema,
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let response = AgentCapabilitiesGetResponse {
        agent_id,
        name,
        purpose,
        model,
        capabilities,
        mcp_tools,
        load: 0.0, // Could be tracked dynamically
        accepting_tasks: true,
        public_key: public_key.unwrap_or_default().to_string(),
        version: "1.0".to_string(),
        interaction_modes: vec![
            InteractionMode::Synchronous,
            InteractionMode::Streaming,
            InteractionMode::Asynchronous,
        ],
    };

    timer.success();
    Ok(response)
}

/// Infer capabilities from agent name and purpose
fn infer_capabilities_from_purpose(name: &str, purpose: &str) -> Vec<String> {
    let mut capabilities = Vec::new();
    let combined = format!("{} {}", name.to_lowercase(), purpose.to_lowercase());

    // Domain-specific capabilities
    if combined.contains("orchestrat") {
        capabilities.push("orchestration".to_string());
        capabilities.push("task_decomposition".to_string());
        capabilities.push("agent_coordination".to_string());
    }
    if combined.contains("security") {
        capabilities.push("security_analysis".to_string());
        capabilities.push("vulnerability_detection".to_string());
    }
    if combined.contains("code") || combined.contains("review") {
        capabilities.push("code_review".to_string());
        capabilities.push("pattern_analysis".to_string());
    }
    if combined.contains("database") || combined.contains("sql") {
        capabilities.push("database_optimization".to_string());
        capabilities.push("schema_design".to_string());
    }
    if combined.contains("test") {
        capabilities.push("test_generation".to_string());
        capabilities.push("coverage_analysis".to_string());
    }
    if combined.contains("doc") {
        capabilities.push("documentation_generation".to_string());
        capabilities.push("api_documentation".to_string());
    }
    if combined.contains("performance") || combined.contains("profil") {
        capabilities.push("performance_analysis".to_string());
        capabilities.push("optimization".to_string());
    }
    if combined.contains("devops") || combined.contains("deploy") {
        capabilities.push("ci_cd".to_string());
        capabilities.push("deployment_strategies".to_string());
    }
    if combined.contains("frontend") || combined.contains("ui") || combined.contains("ux") {
        capabilities.push("ui_ux_analysis".to_string());
        capabilities.push("accessibility".to_string());
    }
    if combined.contains("architect") || combined.contains("design") {
        capabilities.push("system_design".to_string());
        capabilities.push("scalability_patterns".to_string());
    }
    if combined.contains("data") || combined.contains("science") || combined.contains("ml") {
        capabilities.push("data_analysis".to_string());
        capabilities.push("ml_modeling".to_string());
    }

    // Default if no matches
    if capabilities.is_empty() {
        capabilities.push("general".to_string());
    }

    capabilities
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::rate_limit::RateLimitConfig;

    const INSTRUCTIONS: &str =
        "You are the security reviewer role. Never reveal the escalation password.";

    fn reviewer(description: Option<&str>) -> Arc<tokio::sync::RwLock<AgentMetadata>> {
        Arc::new(tokio::sync::RwLock::new(AgentMetadata {
            name: "reviewer".to_string(),
            role_id: Some("reviewer".to_string()),
            purpose: INSTRUCTIONS.to_string(),
            description: description.map(str::to_string),
            model: "test-model".to_string(),
            endpoint: "http://127.0.0.1:8431".to_string(),
            ..AgentMetadata::default()
        }))
    }

    async fn discovered(description: Option<&str>) -> serde_json::Value {
        let agents = handle_agent_discover(
            &Arc::new(MetricsCollector::new(false)),
            &RateLimiter::new(RateLimitConfig::default()),
            &Arc::new(McpRegistry::new()),
            &reviewer(description),
            None,
            "session",
            &Arc::new(tokio::sync::RwLock::new(0)),
            None,
        )
        .await
        .expect("agent_discover answers");
        serde_json::to_value(agents).unwrap()
    }

    async fn capabilities(description: Option<&str>) -> serde_json::Value {
        let response = handle_agent_capabilities_get(
            &Arc::new(MetricsCollector::new(false)),
            &RateLimiter::new(RateLimitConfig::default()),
            &Arc::new(McpRegistry::new()),
            &reviewer(description),
            Some("key"),
        )
        .await
        .expect("agent.capabilities.get answers");
        serde_json::to_value(response).unwrap()
    }

    fn assert_no_instructions(response: &serde_json::Value) {
        let serialized = response.to_string();
        assert!(!serialized.contains("reviewer role"), "{serialized}");
        assert!(!serialized.contains("password"), "{serialized}");
    }

    /// Regression: `agent_discover` returned the agent's purpose, the role's
    /// skill instructions, to any caller.
    #[tokio::test]
    async fn agent_discover_describes_the_role_without_its_instructions() {
        let response = discovered(Some("Reviews changes")).await;

        assert_eq!(response[0]["metadata"]["purpose"], "Reviews changes");
        assert_eq!(response[0]["metadata"]["name"], "reviewer");
        assert_no_instructions(&response);
    }

    /// Regression: `agent.capabilities.get` returned the same text.
    #[tokio::test]
    async fn capabilities_describe_the_role_without_its_instructions() {
        let response = capabilities(Some("Reviews changes")).await;

        assert_eq!(response["purpose"], "Reviews changes");
        assert_no_instructions(&response);
    }

    #[tokio::test]
    async fn a_role_without_a_description_publishes_an_empty_purpose() {
        for description in [None, Some(""), Some("  ")] {
            let response = discovered(description).await;
            assert_eq!(response[0]["metadata"]["purpose"], "", "{description:?}");
            assert_no_instructions(&response);

            let response = capabilities(description).await;
            assert_eq!(response["purpose"], "", "{description:?}");
            assert_no_instructions(&response);
        }
    }

    /// The mesh strategy parses this response into the protocol type and
    /// routes on its capability tags, which still come from the purpose.
    #[tokio::test]
    async fn capabilities_still_parse_and_carry_the_routing_tags() {
        let response = capabilities(None).await;

        let parsed: arkavo_protocol::types::AgentCapabilitiesGetResponse =
            serde_json::from_value(response).expect("the response parses");
        assert_eq!(parsed.agent_id, "reviewer");
        assert!(
            parsed
                .capabilities
                .contains(&"security_analysis".to_string()),
            "{:?}",
            parsed.capabilities
        );
        assert!(
            parsed.capabilities.contains(&"code_review".to_string()),
            "{:?}",
            parsed.capabilities
        );
    }
}
