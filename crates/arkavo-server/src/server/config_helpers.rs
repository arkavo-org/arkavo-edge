use arkavo_protocol::agent_config::parse_agents_config;
use arkavo_protocol::agent_specialization::RoleContext;
use arkavo_protocol::error::{A2aError, Result};
use arkavo_protocol::mcp_registry::McpRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Stash for the most recent SwarmKit role context this agent was specialized into.
///
/// Updated when an `agent.specialize` RPC succeeds; the agent's task
/// loop reads it to tag every tool outcome with `flight_id` +
/// `role_id` so the orchestrator's per-role DecisionTrace receives
/// correctly-attributed events.
///
/// `None` until specialization runs; cleared by future agent_idle (out of
/// scope for this slice).
#[derive(Debug, Default)]
pub struct RoleSpecializationStore {
    inner: RwLock<Option<RoleContext>>,
}

impl RoleSpecializationStore {
    pub async fn set(&self, ctx: RoleContext) {
        *self.inner.write().await = Some(ctx);
    }

    pub async fn get(&self) -> Option<RoleContext> {
        self.inner.read().await.clone()
    }

    pub async fn clear(&self) {
        *self.inner.write().await = None;
    }
}

/// Metadata about the current agent
#[derive(Debug, Clone, Default)]
pub struct AgentMetadata {
    pub name: String,
    pub purpose: String,
    pub model: String,
    pub mode: arkavo_protocol::agent_config::AgentMode,
    pub endpoint: String,
    pub api_keys: std::collections::HashMap<String, String>,
    /// DID:key identifier derived from the agent's device keypair.
    /// Shared across all protocols (A2A, gossip, metrics, UCP).
    pub did: Option<String>,
    /// ES256-signed delegation JWT binding human DID → agent DID.
    pub delegation_jwt: Option<String>,
    /// Entitlements granted via human delegation (from JWT scope claim).
    pub delegated_entitlements: Vec<String>,
    /// Bare MCP tool names this agent is granted by its SwarmKit role
    /// (from `persona.mcp_tools`). Empty for an unspecialized agent (no
    /// filtering applied). When non-empty, the agent loop filters its
    /// `ToolRegistry` to exactly this set — least-privilege (design D9).
    pub granted_tools: Vec<String>,
    /// Whether this agent has been specialized into a SwarmKit role.
    /// Distinguishes "unspecialized" from "specialized with zero tool grants"
    /// so a zero-grant role yields zero tools, not the full registry.
    pub specialized: bool,
    /// Authored per-MTok pricing table received in a specialization bundle.
    /// Applied to the agent's `Router` so its live spend-plane gate prices
    /// cloud arms from the manifest (the pricing home) instead of the built-in
    /// static estimate. Empty for an unspecialized agent → static fallback.
    pub manifest_pricing: Vec<arkavo_budget::provider_costs::PricingEntry>,
}

/// Simple agent configuration structure for validation
#[derive(Debug)]
struct SimpleAgentConfig {
    #[allow(dead_code)]
    name: String,
    purpose: String,
    model: String,
    #[allow(dead_code)]
    listen: String,
}

/// Validate agent configuration content
pub(super) fn validate_agent_config(content: &str) -> std::result::Result<(), String> {
    match parse_simple_agents_config(content) {
        Ok(agents) if agents.is_empty() => Err("No agent configurations found".to_string()),
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Parse error: {e}")),
    }
}

/// Basic parser for agent configuration validation
fn parse_simple_agents_config(
    content: &str,
) -> std::result::Result<Vec<SimpleAgentConfig>, String> {
    let mut agents = Vec::new();
    let mut current_agent: Option<SimpleAgentConfig> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for agent section header
        if trimmed.starts_with("## ") {
            // Save previous agent if exists
            if let Some(agent) = current_agent.take() {
                agents.push(agent);
            }

            let name = trimmed.strip_prefix("## ").unwrap_or("").trim().to_string();
            current_agent = Some(SimpleAgentConfig {
                name,
                purpose: String::new(),
                model: String::new(),
                listen: String::new(),
            });
        } else if let Some(ref mut agent) = current_agent {
            // Parse agent properties
            if let Some((key, value)) = trimmed.split_once(':') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "purpose" => agent.purpose = value.to_string(),
                    "model" => agent.model = value.to_string(),
                    "listen" => agent.listen = value.to_string(),
                    _ => {} // Ignore other fields
                }
            }
        }
    }

    // Save last agent if exists
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}

/// Reload configuration from file watcher
pub(super) async fn reload_configuration_for_watcher(
    content: &str,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
    mcp_registry: Arc<McpRegistry>,
) -> Result<()> {
    // Validate configuration before applying
    if content.trim().is_empty() {
        return Err(A2aError::Configuration(
            "Configuration file is empty".to_string(),
        ));
    }

    // Parse the new configuration
    let agents = parse_agents_config(content)
        .map_err(|e| A2aError::Configuration(format!("Failed to parse configuration: {e}")))?;

    if agents.is_empty() {
        return Err(A2aError::Configuration(
            "No agent configurations found".to_string(),
        ));
    }

    // Find our agent's configuration
    let our_agent_name = agent_metadata.read().await.name.clone();

    let new_config = agents
        .iter()
        .find(|a| a.name == our_agent_name)
        .ok_or_else(|| {
            A2aError::Configuration(format!(
                "Agent '{our_agent_name}' not found in new configuration"
            ))
        })?;

    // Validate required fields
    if new_config.purpose.is_empty() {
        return Err(A2aError::Configuration(
            "Agent purpose cannot be empty".to_string(),
        ));
    }
    if new_config.model.is_empty() {
        return Err(A2aError::Configuration(
            "Agent model cannot be empty".to_string(),
        ));
    }

    // Update metadata
    {
        let mut metadata = agent_metadata.write().await;
        metadata.purpose.clone_from(&new_config.purpose);
        metadata.endpoint.clone_from(&new_config.listen);
        metadata.api_keys.clone_from(&new_config.api_keys);

        info!("Updated agent metadata for '{}'", metadata.name);
        info!("  Purpose: {}", metadata.purpose);
        info!("  Endpoint: {}", metadata.endpoint);
        info!("  API keys updated: {}", metadata.api_keys.len());
    }

    // Handle model changes
    if new_config.model != agent_metadata.read().await.model {
        warn!("Model change detected, but LLM adapter recreation not yet implemented");
        let mut metadata = agent_metadata.write().await;
        metadata.model.clone_from(&new_config.model);
    }

    // Handle MCP server changes
    if !new_config.mcp_servers.is_empty() {
        info!("MCP server configuration changed, clearing existing connections");

        // Clear existing MCP connections
        mcp_registry.clear_connections().await;

        // Log the MCP servers that need to be restarted
        for mcp_server in &new_config.mcp_servers {
            info!("MCP server '{}' needs restart:", mcp_server.name);
            if let Some(cmd) = &mcp_server.command {
                info!("  Command: {} {:?}", cmd, mcp_server.args);
            } else if let Some(url) = &mcp_server.url {
                info!("  URL: {}", url);
            }
        }

        warn!(
            "MCP servers cleared. Manual restart required or use agent restart for full MCP reload"
        );
    }

    Ok(())
}

/// Clean up old configuration backups
pub(super) async fn cleanup_old_backups(backup_dir: &std::path::Path, keep_count: usize) {
    if let Ok(mut entries) = tokio::fs::read_dir(backup_dir).await {
        let mut backups = Vec::new();

        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(metadata) = entry.metadata().await
                && metadata.is_file()
            {
                let filename = entry.file_name().to_string_lossy().to_string();
                if filename.starts_with("AGENTS.md.") && filename.ends_with(".backup") {
                    backups.push((entry.path(), metadata.modified().ok()));
                }
            }
        }

        // Sort by modification time, newest first
        backups.sort_by_key(|b| std::cmp::Reverse(b.1));

        // Remove old backups
        for (path, _) in backups.iter().skip(keep_count) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}
