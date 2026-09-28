use std::sync::Arc;
#[cfg(feature = "claude-agent")]
use tracing::debug;
use tracing::{info, warn};

use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::Router;

/// Lightweight in-process engine for CLI commands (chat, ui).
///
/// Initializes Router + full ToolRegistry without starting a network server.
/// Uses a local runtime (not `'static`) so Metal-backed llama.cpp contexts
/// are dropped deterministically before C++ static destructors.
pub struct LocalEngine {
    router: Arc<Router>,
    tool_registry: Arc<ToolRegistry>,
}

impl LocalEngine {
    /// Create a new LocalEngine with Router and full tool registration.
    pub async fn new() -> Result<Self, String> {
        let router = Router::new()
            .await
            .map_err(|e| format!("Failed to initialize router: {e}"))?;

        // Apply preflight policies from SwarmKit if available
        let agent_config = arkavo_router::load_agent_config().unwrap_or_default();
        let router = if let Some(ref pf) = agent_config.preflight {
            let moderator = arkavo_router::build_moderator_from_config(pf);
            let count = moderator.len();
            if count > 0 {
                info!("Preflight: {} policies loaded", count);
            }
            router.with_preflight(moderator)
        } else {
            router
        };

        // Spend plane: adopt the configured cloud policy (shared resolver), and
        // share one budget tracker between the router (live-cap enforcement) and
        // UCP.
        let cloud_policy = crate::spend_plane::cloud_policy_from_config(&agent_config);
        let budget_tracker =
            arkavo_budget::BudgetTracker::new(arkavo_budget::BudgetConfig::default())
                .await
                .ok()
                .map(Arc::new);
        let router = router.with_cloud_policy(cloud_policy);
        let router = match budget_tracker {
            Some(ref tracker) => router.with_budget_tracker(tracker.clone()),
            None => router,
        };

        let router = Arc::new(router);
        let tool_registry = Arc::new(Self::build_tool_registry(&router, budget_tracker).await);

        Ok(Self {
            router,
            tool_registry,
        })
    }

    /// Get the router for model selection and inference.
    pub fn router(&self) -> Arc<Router> {
        self.router.clone()
    }

    /// Get the tool registry with all registered tools.
    pub fn tool_registry(&self) -> Arc<ToolRegistry> {
        self.tool_registry.clone()
    }

    /// Build a ToolRegistry with all available tool sets.
    async fn build_tool_registry(
        router: &Arc<Router>,
        budget_tracker: Option<Arc<arkavo_budget::BudgetTracker>>,
    ) -> ToolRegistry {
        // Start with built-in tools (time, filesystem, etc.)
        let storage = match arkavo_memory::storage::MemoryStorage::new().await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!("Memory storage unavailable: {e}, using empty registry");
                return ToolRegistry::empty();
            }
        };
        let mut registry = ToolRegistry::new(storage);

        // Router tools (list_models)
        arkavo_router::tools::register_tools(&mut registry, router.clone());

        // Mesh orchestration tools
        let mesh_state = Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
        arkavo_mcp_mesh::register_tools(&mut registry, mesh_state);

        // HRM orchestration tools (Conductor API)
        let hrm_state = Arc::new(tokio::sync::RwLock::new(
            arkavo_hrm::tools::HrmToolsState::new(),
        ));
        arkavo_hrm::tools::register_tools(&mut registry, hrm_state);

        // UCP payment tools — reuse the router's budget tracker so spend
        // accounting is unified; fall back to a fresh default tracker.
        let ucp_budget_tracker = match budget_tracker {
            Some(tracker) => Some(tracker),
            None => arkavo_budget::BudgetTracker::new(arkavo_budget::BudgetConfig::default())
                .await
                .map(Arc::new)
                .map_err(|e| warn!("UCP tools disabled: {e}"))
                .ok(),
        };
        if let Some(tracker) = ucp_budget_tracker {
            let ucp_state = Arc::new(tokio::sync::RwLock::new(arkavo_ucp::UcpState::new(tracker)));
            arkavo_ucp::register_tools(&mut registry, ucp_state);
        }

        // Claude Agent SDK tools (feature-gated)
        #[cfg(feature = "claude-agent")]
        {
            Self::register_claude_tools(&mut registry).await;
        }

        info!("LocalEngine tool registry built");
        registry
    }

    #[cfg(feature = "claude-agent")]
    async fn register_claude_tools(registry: &mut ToolRegistry) {
        use arkavo_mcp_claude::{ClaudeCodeCapability, ClaudeCodeConfig};

        let config = ClaudeCodeConfig::default();
        if !config.enabled || !arkavo_mcp_claude::is_auth_available() {
            if config.enabled {
                debug!("Claude tools skipped: no API key or cached OAuth token");
            }
            return;
        }

        let event_writer = Arc::new(arkavo_events::EventWriter::new(
            arkavo_events::EventWriterConfig::default(),
        ));
        match ClaudeCodeCapability::new(
            config,
            "arkavo-local".to_string(),
            event_writer,
            None,
            None,
        ) {
            Ok(capability) => {
                let capability = Arc::new(capability);
                if let Err(e) = capability.prepare().await {
                    warn!("Claude tools not ready: {e}");
                } else {
                    arkavo_mcp_claude::register_tools(registry, capability);
                    info!("Claude MCP tools registered");
                }
            }
            Err(e) => {
                warn!("Claude tools disabled: {e}");
            }
        }
    }
}
