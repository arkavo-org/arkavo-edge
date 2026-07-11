use arkavo_protocol::agent_specialization::RoleContext;
use arkavo_protocol::error::{A2aError, Result};
use arkavo_protocol::mcp_registry::McpRegistry;
use std::path::{Path, PathBuf};
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

/// Default location for a newly created kit file when the server starts
/// with no kit configured yet. Mirrors the historical default write
/// location of `.arkavo/AGENTS.md`.
pub(super) const DEFAULT_KIT_PATH: &str = ".arkavo/agent.swarmkit.yaml";

/// Resolve the server's SwarmKit kit file path using the same discovery
/// order the boot path uses (`arkavo_router::load_agent_config` →
/// `arkavo_swarmkit::load_discovered_kit`): `ARKAVO_SWARMKIT_PATH`, then
/// `.arkavo/*.swarmkit.yaml`, then cwd. Returns `None` when no kit exists
/// yet (fresh install), or discovery is ambiguous/unsupported (multiple
/// kits, AGENTS.md-only) — callers decide the missing-kit policy (`get`
/// errors, `update` creates at [`DEFAULT_KIT_PATH`]).
pub(super) fn resolve_kit_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    resolve_kit_path_in(&cwd)
}

/// [`resolve_kit_path`] with an explicit working directory, so callers
/// (tests included) don't depend on the process's current directory.
pub(super) fn resolve_kit_path_in(cwd: &Path) -> Option<PathBuf> {
    arkavo_swarmkit::discover_kit_path(cwd).ok()
}

/// Basename of a resolved kit path, used as the backup-file naming prefix
/// (`<kit_filename>.<timestamp>.backup`).
pub(super) fn kit_filename_of(kit_path: &Path) -> String {
    kit_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "kit".to_string())
}

/// Validate configuration content as a SwarmKit kit manifest.
pub(super) fn validate_kit_yaml(content: &str) -> std::result::Result<(), String> {
    arkavo_swarmkit::parse_yaml(content)
        .map(|_| ())
        .map_err(|e| format!("Kit parse error: {e}"))
}

/// Update [`AgentMetadata`] from a SwarmKit kit's primary role, then handle
/// the kit-level MCP server list the same way the legacy AGENTS.md reload
/// did (clear stale connections, log servers needing a restart).
///
/// Shared by the `agent.config.update` RPC path and the kit file watcher —
/// both hot-reload the same file, just from different triggers. Does not
/// touch the filesystem: callers supply already-read file content.
pub(super) async fn apply_kit_reload(
    content: &str,
    agent_metadata: &Arc<RwLock<AgentMetadata>>,
    mcp_registry: &McpRegistry,
) -> Result<()> {
    if content.trim().is_empty() {
        return Err(A2aError::Configuration("Kit file is empty".to_string()));
    }

    let manifest = arkavo_swarmkit::parse_yaml(content)
        .map_err(|e| A2aError::Configuration(format!("Failed to parse kit: {e}")))?;
    let runtime_config = arkavo_swarmkit::agent_runtime_config_from_manifest(&manifest);

    if runtime_config.objective_goal.trim().is_empty() {
        return Err(A2aError::Configuration(
            "Kit objective.goal cannot be empty".to_string(),
        ));
    }

    {
        let mut metadata = agent_metadata.write().await;
        metadata.purpose = runtime_config.purpose_text();
        if let Some(listen) = &runtime_config.runtime.listen {
            metadata.endpoint.clone_from(listen);
        }

        info!("Updated agent metadata for '{}'", metadata.name);
        info!("  Purpose: {}", metadata.purpose);
        info!("  Endpoint: {}", metadata.endpoint);
    }

    if !runtime_config.runtime.mcp_servers.is_empty() {
        info!("MCP server configuration changed, clearing existing connections");
        mcp_registry.clear_connections().await;

        for mcp_server in &runtime_config.runtime.mcp_servers {
            info!("MCP server '{}' needs restart:", mcp_server.name);
            if let Some(cmd) = &mcp_server.command {
                info!("  Command: {} {:?}", cmd, mcp_server.args);
            } else if let Some(url) = &mcp_server.url {
                info!("  URL: {}", url);
            }
        }

        warn!("MCP servers cleared. Manual restart required for full MCP reload");
    }

    info!("Kit hot-reload completed successfully");
    Ok(())
}

/// Reload configuration from the kit file watcher.
pub(super) async fn reload_configuration_for_watcher(
    content: &str,
    agent_metadata: Arc<RwLock<AgentMetadata>>,
    mcp_registry: Arc<McpRegistry>,
) -> Result<()> {
    apply_kit_reload(content, &agent_metadata, &mcp_registry).await
}

/// Clean up old configuration backups for the given kit filename.
pub(super) async fn cleanup_old_backups(
    backup_dir: &std::path::Path,
    keep_count: usize,
    kit_filename: &str,
) {
    if let Ok(mut entries) = tokio::fs::read_dir(backup_dir).await {
        let mut backups = Vec::new();
        let prefix = format!("{kit_filename}.");

        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(metadata) = entry.metadata().await
                && metadata.is_file()
            {
                let filename = entry.file_name().to_string_lossy().to_string();
                if filename.starts_with(&prefix) && filename.ends_with(".backup") {
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

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_KIT_YAML: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#;

    #[test]
    fn validate_kit_yaml_accepts_valid_kit() {
        assert!(validate_kit_yaml(MINIMAL_KIT_YAML).is_ok());
    }

    #[test]
    fn validate_kit_yaml_rejects_invalid_yaml() {
        let err = validate_kit_yaml("not: valid: yaml: at: all:").unwrap_err();
        assert!(err.contains("Kit parse error"));
    }

    #[test]
    fn resolve_kit_path_in_finds_single_kit_in_arkavo_dir() {
        let dir = tempfile::tempdir().unwrap();
        let arkavo_dir = dir.path().join(".arkavo");
        std::fs::create_dir_all(&arkavo_dir).unwrap();
        std::fs::write(arkavo_dir.join("agent.swarmkit.yaml"), MINIMAL_KIT_YAML).unwrap();

        let path = resolve_kit_path_in(dir.path()).expect("kit should be discovered");
        assert!(path.ends_with("agent.swarmkit.yaml"));
    }

    #[test]
    fn resolve_kit_path_in_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_kit_path_in(dir.path()).is_none());
    }

    #[tokio::test]
    async fn apply_kit_reload_updates_purpose_from_kit() {
        let agent_metadata = Arc::new(RwLock::new(AgentMetadata {
            name: "agent".to_string(),
            ..Default::default()
        }));
        let mcp_registry = McpRegistry::new();

        apply_kit_reload(MINIMAL_KIT_YAML, &agent_metadata, &mcp_registry)
            .await
            .expect("valid kit should reload");

        let metadata = agent_metadata.read().await;
        assert_eq!(metadata.purpose, "say hello");
    }

    #[tokio::test]
    async fn apply_kit_reload_rejects_empty_content() {
        let agent_metadata = Arc::new(RwLock::new(AgentMetadata::default()));
        let mcp_registry = McpRegistry::new();

        let err = apply_kit_reload("   ", &agent_metadata, &mcp_registry)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[tokio::test]
    async fn apply_kit_reload_rejects_invalid_yaml() {
        let agent_metadata = Arc::new(RwLock::new(AgentMetadata::default()));
        let mcp_registry = McpRegistry::new();

        let err = apply_kit_reload("not: valid: yaml: at: all:", &agent_metadata, &mcp_registry)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to parse kit"));
    }
}
