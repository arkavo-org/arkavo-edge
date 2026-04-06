use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use arkavo_events::{Event, EventPayload, EventWriter, EventWriterConfig};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_llm::LlmClientAdapter;
#[cfg(feature = "llama-cpp")]
use arkavo_llm::ModelRegistry;
use jsonrpsee::server::middleware::http::ProxyGetRequestLayer;
use jsonrpsee::server::{ServerBuilder, ServerHandle};

// SECURITY FIX (CRI-001): NoOpAuthBackend removed
// Use JwtAuthBackend or MultiAuthBackend for production
use arkavo_protocol::chat_session;
use arkavo_protocol::config::{BufferConfig, ServerConfig};
use arkavo_protocol::error::{A2aError, Result};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};

use super::config_helpers::{AgentMetadata, reload_configuration_for_watcher};
use super::learning_bus::LearningBus;
use super::startup::{AgentPlan, run_startup_planning_phase};
use super::tool_memory::ToolMemory;
use super::{A2aRpcImpl, A2aRpcServer};

#[cfg(feature = "kas")]
use arkavo_tdf::KasA2aHandler;

pub struct A2aServer {
    config: ServerConfig,
    buffer_config: BufferConfig,
    mcp_registry: Arc<McpRegistry>,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
    llm_adapter: Arc<tokio::sync::RwLock<Option<Arc<LlmClientAdapter>>>>,
    /// Model registry for multi-model inference support
    #[cfg(feature = "llama-cpp")]
    model_registry: Arc<tokio::sync::RwLock<Option<Arc<ModelRegistry>>>>,
    router: Arc<tokio::sync::RwLock<Option<Arc<arkavo_router::Router>>>>,
    tool_registry: Arc<tokio::sync::RwLock<Option<Arc<arkavo_mcp_tools::ToolRegistry>>>>,
    event_writer: Arc<tokio::sync::RwLock<Option<Arc<EventWriter>>>>,
    session_id: String,
    event_sequence: Arc<tokio::sync::RwLock<u64>>,
    file_watcher_handle: Arc<tokio::sync::RwLock<Option<tokio::task::JoinHandle<()>>>>,
    #[allow(dead_code)]
    #[allow(clippy::type_complexity)]
    mcp_reload_callback: Arc<tokio::sync::RwLock<Option<Arc<dyn Fn() + Send + Sync>>>>,
    conductor: Arc<tokio::sync::RwLock<Arc<Conductor<InMemoryTaskStore>>>>,
    agent_plan: Arc<tokio::sync::RwLock<AgentPlan>>,
    planning_completed: Arc<std::sync::atomic::AtomicBool>,
    agent_memory: Arc<tokio::sync::RwLock<ToolMemory>>,
    /// Learning bus for gossip-based learning propagation
    learning_bus: Arc<tokio::sync::RwLock<Option<Arc<LearningBus>>>>,
    /// Base64-encoded ECDSA P-256 public key for TDF encryption
    public_key: Arc<tokio::sync::RwLock<Option<String>>>,
    /// Budget manager loaded from AGENTS.md config
    budget_manager: Arc<tokio::sync::RwLock<Option<Arc<arkavo_budget::BudgetManager>>>>,
    /// Cached AGENTS.md config to avoid repeated file reads
    agent_config: Arc<tokio::sync::RwLock<arkavo_router::AgentConfig>>,
    /// Federated memory service for ABAC-scoped cross-agent retrieval
    federated_memory: Arc<tokio::sync::RwLock<Option<Arc<arkavo_memory::FederatedMemoryService>>>>,
    /// Agent cycle counter shared with the RPC impl for learning/status
    orchestrator_tick: Arc<std::sync::atomic::AtomicU64>,
    /// Per-agent compute budget (specialists check before each cycle)
    compute_budget: arkavo_budget::SharedComputeBudget,
    /// Total system RAM detected at startup (bytes)
    total_ram_bytes: u64,
    /// Mesh state for A2A peer delegation (shared between orchestrator and RPC)
    mesh_state: Arc<arkavo_mcp_mesh::MeshToolsState>,
    /// Event sender for injecting A2A messages into the agent loop
    agent_event_tx:
        Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<super::agent_event::AgentEvent>>>>,
    /// Shared Iroh P2P node for TDF blob transport (one per agent)
    #[cfg(feature = "iroh")]
    iroh_node: Arc<tokio::sync::RwLock<Option<Arc<arkavo_tdf_iroh::IrohNode>>>>,
}

impl A2aServer {
    pub fn new(config: ServerConfig) -> Self {
        Self::with_buffer_config(config, BufferConfig::default())
    }

    pub fn with_buffer_config(config: ServerConfig, buffer_config: BufferConfig) -> Self {
        Self {
            config,
            buffer_config,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
            llm_adapter: Arc::new(tokio::sync::RwLock::new(None)),
            #[cfg(feature = "llama-cpp")]
            model_registry: Arc::new(tokio::sync::RwLock::new(None)),
            router: Arc::new(tokio::sync::RwLock::new(None)),
            tool_registry: Arc::new(tokio::sync::RwLock::new(None)),
            event_writer: Arc::new(tokio::sync::RwLock::new(None)),
            session_id: uuid::Uuid::new_v4().to_string(),
            event_sequence: Arc::new(tokio::sync::RwLock::new(0)),
            file_watcher_handle: Arc::new(tokio::sync::RwLock::new(None)),
            mcp_reload_callback: Arc::new(tokio::sync::RwLock::new(None)),
            conductor: Arc::new(tokio::sync::RwLock::new(Arc::new(Conductor::new(
                InMemoryTaskStore::new(),
            )))),
            agent_plan: Arc::new(tokio::sync::RwLock::new(AgentPlan::default())),
            planning_completed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            agent_memory: Arc::new(tokio::sync::RwLock::new(ToolMemory::new(10))),
            learning_bus: Arc::new(tokio::sync::RwLock::new(None)),
            public_key: Arc::new(tokio::sync::RwLock::new(None)),
            budget_manager: Arc::new(tokio::sync::RwLock::new(None)),
            agent_config: Arc::new(tokio::sync::RwLock::new(
                arkavo_router::load_agent_config().unwrap_or_default(),
            )),
            federated_memory: Arc::new(tokio::sync::RwLock::new(None)),
            orchestrator_tick: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            compute_budget: arkavo_budget::new_shared_compute_budget(),
            total_ram_bytes: {
                let mut sys = sysinfo::System::new();
                sys.refresh_memory();
                sys.total_memory()
            },
            mesh_state: Arc::new(arkavo_mcp_mesh::MeshToolsState::new()),
            agent_event_tx: Arc::new(tokio::sync::Mutex::new(None)),
            #[cfg(feature = "iroh")]
            iroh_node: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Set the agent's public key for TDF encryption
    pub async fn set_public_key(&self, key: String) {
        *self.public_key.write().await = Some(key);
    }

    /// Get the agent's public key
    pub async fn public_key(&self) -> Option<String> {
        self.public_key.read().await.clone()
    }

    pub fn mcp_registry(&self) -> Arc<McpRegistry> {
        self.mcp_registry.clone()
    }

    /// Get the shared Iroh P2P node (if started).
    #[cfg(feature = "iroh")]
    pub async fn iroh_node(&self) -> Option<Arc<arkavo_tdf_iroh::IrohNode>> {
        self.iroh_node.read().await.clone()
    }

    /// Set the learning bus for gossip-based learning
    pub async fn set_learning_bus(&self, bus: Arc<LearningBus>) {
        *self.learning_bus.write().await = Some(bus);
    }

    /// Get the learning bus reference
    pub async fn learning_bus(&self) -> Option<Arc<LearningBus>> {
        self.learning_bus.read().await.clone()
    }

    /// Initialize the model registry for multi-model support
    #[cfg(feature = "llama-cpp")]
    pub async fn initialize_model_registry(&self) -> Arc<ModelRegistry> {
        let registry = Arc::new(ModelRegistry::new());
        *self.model_registry.write().await = Some(registry.clone());
        info!("Model registry initialized for multi-model support");
        registry
    }

    /// Get the model registry if initialized
    #[cfg(feature = "llama-cpp")]
    pub async fn model_registry(&self) -> Option<Arc<ModelRegistry>> {
        self.model_registry.read().await.clone()
    }

    /// Load a model into the registry
    #[cfg(feature = "llama-cpp")]
    pub async fn load_model(&self, name: &str, path: &str) -> Result<()> {
        let registry = self.model_registry.read().await.clone();
        let registry = registry
            .ok_or_else(|| A2aError::Configuration("Model registry not initialized".to_string()))?;

        registry
            .load(name, path)
            .map_err(|e| A2aError::Configuration(format!("Failed to load model: {e}")))?;

        info!("Model '{}' loaded from '{}' into registry", name, path);
        Ok(())
    }

    /// Unload a model from the registry
    #[cfg(feature = "llama-cpp")]
    pub async fn unload_model(&self, name: &str) -> bool {
        let registry = self.model_registry.read().await.clone();
        if let Some(registry) = registry {
            let removed = registry.unload_model(name);
            if removed {
                info!("Model '{}' unloaded from registry", name);
            }
            removed
        } else {
            false
        }
    }

    /// Get list of loaded models
    #[cfg(feature = "llama-cpp")]
    pub async fn list_loaded_models(&self) -> Vec<String> {
        let registry = self.model_registry.read().await.clone();
        registry.map(|r| r.model_names()).unwrap_or_default()
    }

    pub async fn set_agent_metadata(
        &self,
        name: String,
        purpose: String,
        model: String,
        mode: arkavo_protocol::agent_config::AgentMode,
        did: Option<String>,
    ) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.name.clone_from(&name);
        metadata.purpose = purpose;
        metadata.model.clone_from(&model);
        metadata.mode = mode;
        metadata.endpoint = format!("http://{}:{}", self.config.bind_address, self.config.port);
        metadata.did = did;
        drop(metadata);

        // Always initialize router for task execution via HRM conductor
        // Router is required by execute_with_conductor regardless of model
        info!("Initializing router for task execution");
        self.initialize_router().await;
        self.build_tool_registry().await;

        // LLM inference is handled by the Router's embedded llama.cpp via
        // route_with_tools_hinted(). No separate LLM adapter needed.
    }

    pub async fn set_api_keys(&self, api_keys: std::collections::HashMap<String, String>) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.api_keys = api_keys;
    }

    #[allow(dead_code)]
    async fn reload_configuration_from_content(&self, content: &str) -> Result<()> {
        use arkavo_protocol::agent_config::parse_agents_config;

        let configs = parse_agents_config(content)
            .map_err(|e| A2aError::Configuration(format!("Failed to parse config: {e}")))?;

        let current_name = {
            let metadata = self.agent_metadata.read().await;
            metadata.name.clone()
        };

        let new_config = configs
            .iter()
            .find(|c| c.name == current_name)
            .ok_or_else(|| {
                A2aError::Configuration(format!(
                    "Agent '{current_name}' not found in updated configuration"
                ))
            })?;

        info!(
            "Updating agent metadata: name={}, purpose={}, model={}",
            new_config.name, new_config.purpose, new_config.model
        );

        let (model_changed, existing_did) = {
            let metadata = self.agent_metadata.read().await;
            (metadata.model != new_config.model, metadata.did.clone())
        };

        self.set_agent_metadata(
            new_config.name.clone(),
            new_config.purpose.clone(),
            new_config.model.clone(),
            new_config.mode.clone(),
            existing_did,
        )
        .await;

        if !new_config.api_keys.is_empty() {
            info!("Updating API keys");
            self.set_api_keys(new_config.api_keys.clone()).await;
        }

        if model_changed {
            info!("Model changed, LLM adapter recreated");
        }

        if !new_config.mcp_servers.is_empty() {
            warn!(
                "MCP server configuration changes detected. Full hot-reload for MCP servers will be implemented in Phase 3."
            );
            warn!("For now, MCP server changes require agent restart to take effect.");
        }

        let current_listen = format!("{}:{}", self.config.bind_address, self.config.port);
        if new_config.listen != current_listen && new_config.listen != "0.0.0.0:0" {
            warn!(
                "Listen address changes require agent restart to take effect (current: {}, new: {})",
                current_listen, new_config.listen
            );
        }

        info!("Configuration reloaded successfully");
        Ok(())
    }

    pub async fn initialize_event_writer(&self) -> Result<()> {
        use arkavo_events::writer::EventWriterBuilder;
        use std::time::Duration;

        let config = EventWriterConfig {
            buffer_size: 10_000,
            flush_interval: Duration::from_millis(100),
            batch_size: 200,
        };

        let writer = EventWriterBuilder::new()
            .with_config(config)
            .add_handler(move |events| {
                for event in events {
                    tracing::debug!(
                        event_type = %event.event_type(),
                        session_id = %event.session_id,
                        "Event captured"
                    );
                }
            })
            .build();

        *self.event_writer.write().await = Some(Arc::new(writer));
        self.emit_session_started().await?;
        Ok(())
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    async fn next_sequence(&self) -> u64 {
        let mut seq = self.event_sequence.write().await;
        let current = *seq;
        *seq += 1;
        current
    }

    async fn emit_session_started(&self) -> Result<()> {
        if let Some(writer) = self.event_writer.read().await.as_ref() {
            let metadata = self.agent_metadata.read().await;
            let capabilities = vec![
                "a2a-protocol".to_string(),
                "mcp-integration".to_string(),
                "chat-streaming".to_string(),
            ];

            let sequence = self.next_sequence().await;
            let event = Event::new(
                self.session_id.clone(),
                sequence,
                metadata.name.clone(),
                EventPayload::SessionStarted {
                    capabilities: Some(capabilities),
                    metadata: Some(
                        [
                            (
                                "model".to_string(),
                                serde_json::Value::String(metadata.model.clone()),
                            ),
                            (
                                "purpose".to_string(),
                                serde_json::Value::String(metadata.purpose.clone()),
                            ),
                            (
                                "endpoint".to_string(),
                                serde_json::Value::String(metadata.endpoint.clone()),
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                },
            );

            writer.write(event).await.map_err(|e| {
                A2aError::Internal(format!("Failed to write session started event: {e}"))
            })?;
        }
        Ok(())
    }

    /// Resolve model hint from AGENTS.md.
    ///
    /// Returns the model specified in the agent's configuration. The router's
    /// ModelSelector memory budget (set from hint size) prevents Thompson
    /// Sampling from exploring models far beyond this hint.
    async fn resolve_model_hint(&self) -> Option<arkavo_router::ModelChoice> {
        let metadata = self.agent_metadata.read().await;
        let choice = arkavo_router::ModelChoice::from_name(&metadata.model)?;
        Some(choice)
    }

    async fn initialize_router(&self) {
        // Check for offline mode (local models only)
        let offline_mode = std::env::var("ARKAVO_OFFLINE").is_ok();
        if offline_mode {
            info!("Initializing router in OFFLINE mode (local models only)");
        } else {
            info!("Initializing router for dynamic model selection");
        }

        // Use cached AGENTS.md config for preflight, budget, KAS
        let agent_config = self.agent_config.read().await.clone();

        let router_result = if offline_mode {
            arkavo_router::Router::new_offline().await
        } else {
            arkavo_router::Router::new().await
        };

        match router_result {
            Ok(router) => {
                // Wire preflight from AGENTS.md
                let router = if let Some(ref pf) = agent_config.preflight {
                    let moderator = arkavo_router::build_moderator_from_config(pf);
                    let count = moderator.len();
                    if count > 0 {
                        info!("Preflight: {} policies loaded from AGENTS.md", count);
                    }
                    router.with_preflight(moderator)
                } else {
                    router
                };

                // Attach advisor persistence (SQLite-backed learned adjustments)
                let router = match Self::create_advisor_store().await {
                    Some(store) => {
                        info!("✓ Advisor persistence enabled");
                        router.with_advisor_store(store).await
                    }
                    None => router,
                };

                // Wire TDF audit encryption from AGENTS.md
                #[cfg(feature = "kas")]
                let router = if let Some(ref kas) = agent_config.kas {
                    if kas.enabled {
                        // Each agent is its own KAS -- use local endpoint, not cloud SaaS
                        let local_kas_url =
                            format!("http://{}:{}", self.config.bind_address, self.config.port);
                        let tdf_config = arkavo_router::TdfAuditConfig {
                            kas_url: local_kas_url,
                            agent_id: agent_config
                                .name
                                .clone()
                                .unwrap_or_else(|| "unknown-agent".to_string()),
                            key_id: kas
                                .key_id
                                .clone()
                                .unwrap_or_else(|| "kas-key-1".to_string()),
                        };
                        let encryptor = arkavo_router::MessageEncryptor::new(tdf_config);
                        info!("TDF audit encryption enabled for cloud-bound prompts");
                        let router = router.with_tdf_encryptor(encryptor);
                        if let Some(store) = Self::create_tdf_audit_store().await {
                            info!("✓ TDF audit persistence enabled");
                            router.with_tdf_audit_store(store)
                        } else {
                            router
                        }
                    } else {
                        router
                    }
                } else {
                    router
                };

                let router = Arc::new(router);

                // Set per-agent memory budget from the model hint so Thompson
                // Sampling only considers models at or below the configured size.
                // Cap = hint size exactly. This prevents loading oversized models
                // (e.g. qwen3.5-27b at 23 GB when hint is glm-4.7-flash at 20 GB).
                {
                    let metadata = self.agent_metadata.read().await;
                    if let Some(hint) = arkavo_router::ModelChoice::from_name(&metadata.model) {
                        let hint_bytes = hint.size_bytes();
                        if hint_bytes > 0 {
                            let cap = hint_bytes;
                            router.set_memory_budget(cap);
                            info!(
                                model = hint.name(),
                                hint_mb = hint_bytes / (1024 * 1024),
                                budget_mb = cap / (1024 * 1024),
                                "Router model budget set from AGENTS.md hint"
                            );
                        }
                    }
                }

                *self.router.write().await = Some(router.clone());
                info!(
                    "✓ Successfully initialized router (offline={})",
                    offline_mode
                );

                // Wire budget from AGENTS.md
                if let Some(ref budget_yaml) = agent_config.budget {
                    let budget_config = build_budget_config(budget_yaml);
                    match arkavo_budget::BudgetManager::new(budget_config).await {
                        Ok(manager) => {
                            *self.budget_manager.write().await = Some(Arc::new(manager));
                            info!("Budget enforcement loaded from AGENTS.md");
                        }
                        Err(e) => {
                            warn!("Failed to initialize budget manager: {}", e);
                        }
                    }
                }

                // Set router on learning bus for LLM-based synthesis
                if let Some(bus) = self.learning_bus.read().await.as_ref() {
                    bus.set_router(router.clone()).await;
                    info!("✓ Router configured for learning synthesis");

                    // Load AGENTS.md lessons into policy cache at startup
                    if let Some(ref name) = agent_config.name {
                        // Use the purpose field as AGENTS.md content if available
                        let agents_md_content = Self::read_agents_md_content().await;
                        if !agents_md_content.is_empty() {
                            bus.load_agents_md_lessons(&agents_md_content).await;
                        }
                        let _ = name; // used for agent_config context
                    }
                }

                // Initialize federated memory service for cross-agent retrieval
                if let Some(svc) = Self::create_federated_memory_service().await {
                    *self.federated_memory.write().await = Some(svc);
                }
            }
            Err(e) => {
                error!(error = %e, "✗ Failed to initialize router");
            }
        }
    }

    /// Read AGENTS.md content from disk for teaching lesson injection.
    async fn read_agents_md_content() -> String {
        let path = if std::path::Path::new(".arkavo/AGENTS.md").exists() {
            ".arkavo/AGENTS.md"
        } else if std::path::Path::new("AGENTS.md").exists() {
            "AGENTS.md"
        } else {
            return String::new();
        };
        tokio::fs::read_to_string(path).await.unwrap_or_default()
    }

    /// Create the advisor state store alongside the workspace memory DB.
    async fn create_advisor_store() -> Option<arkavo_memory::AdvisorStateStore> {
        let db_path = if std::path::Path::new(".arkavo").exists() {
            std::path::PathBuf::from(".arkavo/memory_server/advisor.db")
        } else {
            return None;
        };

        match arkavo_memory::AdvisorStateStore::new(&db_path).await {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!("Advisor persistence unavailable: {e}");
                None
            }
        }
    }

    /// Create the TDF audit store for persisting encryption manifests.
    #[cfg(feature = "kas")]
    async fn create_tdf_audit_store() -> Option<arkavo_memory::TdfAuditStore> {
        let db_path = if std::path::Path::new(".arkavo").exists() {
            std::path::PathBuf::from(".arkavo/memory_server/tdf_audit.db")
        } else {
            return None;
        };

        match arkavo_memory::TdfAuditStore::new(&db_path).await {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!("TDF audit persistence unavailable: {e}");
                None
            }
        }
    }

    /// Create the federated memory service for ABAC-scoped cross-agent retrieval.
    async fn create_federated_memory_service() -> Option<Arc<arkavo_memory::FederatedMemoryService>>
    {
        let db_path = if std::path::Path::new(".arkavo").exists() {
            std::path::PathBuf::from(".arkavo/memory_server/federated.db")
        } else {
            return None;
        };

        match arkavo_memory::FederatedMemoryService::new(&db_path).await {
            Ok(svc) => {
                info!("✓ Federated memory service enabled");
                Some(Arc::new(svc))
            }
            Err(e) => {
                tracing::warn!("Federated memory unavailable: {e}");
                None
            }
        }
    }

    async fn build_tool_registry(&self) {
        info!("Building tool registry from MCP connections");
        // Use empty registry - agents only get tools from their configured MCP servers
        // This enables small models (ministral-3b) to work with focused tool sets
        let mut tool_registry = arkavo_mcp_tools::ToolRegistry::empty();

        // Project MCP tools into the registry
        match self.mcp_registry.list_all_tools().await {
            Ok(tools) => {
                info!("Projecting {} tools from MCP servers", tools.len());
                for tool in tools {
                    info!("  - {} (from MCP)", tool.name);
                }
            }
            Err(e) => {
                warn!("Failed to list tools from MCP servers: {}", e);
            }
        }

        // Register router tools (list_models) for model discovery
        if let Some(router) = self.router.read().await.as_ref() {
            arkavo_router::tools::register_tools(&mut tool_registry, router.clone());
        }

        // Register mesh orchestration tools for agent coordination
        let mesh_state = Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
        arkavo_mcp_mesh::register_tools(&mut tool_registry, mesh_state);

        // Register HRM orchestration tools (Conductor API)
        let hrm_state = Arc::new(tokio::sync::RwLock::new(
            arkavo_hrm::tools::HrmToolsState::new(),
        ));
        arkavo_hrm::tools::register_tools(&mut tool_registry, hrm_state);

        // Register UCP payment tools for AI agent commerce
        match arkavo_budget::BudgetTracker::new(arkavo_budget::BudgetConfig::default()).await {
            Ok(tracker) => {
                let ucp_budget_tracker = Arc::new(tracker);
                let ucp_state = Arc::new(tokio::sync::RwLock::new(arkavo_ucp::UcpState::new(
                    ucp_budget_tracker,
                )));
                arkavo_ucp::register_tools(&mut tool_registry, ucp_state);
            }
            Err(e) => {
                warn!("UCP tools disabled: failed to create budget tracker: {e}");
            }
        }

        // Register Claude Agent SDK tools (entitlement-gated, requires OAuth)
        #[cfg(feature = "claude-agent")]
        {
            use arkavo_mcp_claude::{ClaudeCodeCapability, ClaudeCodeConfig};

            let claude_config = ClaudeCodeConfig::default();
            if claude_config.enabled && arkavo_mcp_claude::is_auth_available() {
                let event_writer = Arc::new(EventWriter::new(EventWriterConfig::default()));
                match ClaudeCodeCapability::new(
                    claude_config,
                    "arkavo-server".to_string(),
                    event_writer,
                    None,
                    None,
                ) {
                    Ok(capability) => {
                        let capability = Arc::new(capability);
                        if let Err(e) = capability.prepare().await {
                            warn!("Claude MCP tools not ready: {e}");
                        } else {
                            arkavo_mcp_claude::register_tools(&mut tool_registry, capability);
                            info!("Claude MCP tools registered");
                        }
                    }
                    Err(e) => {
                        warn!("Claude MCP tools disabled: {e}");
                    }
                }
            } else if claude_config.enabled {
                debug!("Claude MCP tools skipped: no API key or cached OAuth token");
            }
        }

        *self.tool_registry.write().await = Some(Arc::new(tool_registry));
        info!("✓ Tool registry built");
    }

    pub async fn start_file_watcher(&self) -> Result<()> {
        use notify::{Event, EventKind, RecursiveMode, Watcher};
        use std::sync::mpsc::channel;
        use std::time::Duration;

        self.stop_file_watcher().await;

        let config_path = if std::path::Path::new(".arkavo/AGENTS.md").exists() {
            std::path::Path::new(".arkavo/AGENTS.md")
        } else if std::path::Path::new("AGENTS.md").exists() {
            std::path::Path::new("AGENTS.md")
        } else {
            info!("AGENTS.md not found, skipping file watcher setup");
            return Ok(());
        };

        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(tx)
            .map_err(|e| A2aError::Internal(format!("Failed to create file watcher: {e}")))?;

        watcher
            .watch(config_path, RecursiveMode::NonRecursive)
            .map_err(|e| A2aError::Internal(format!("Failed to watch AGENTS.md: {e}")))?;

        info!("File watcher started for {:?}", config_path);

        let agent_metadata = self.agent_metadata.clone();
        let mcp_registry = self.mcp_registry.clone();

        let handle = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            let mut last_reload = std::time::Instant::now();
            let _watcher = watcher;

            loop {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(Ok(Event {
                        kind: EventKind::Modify(_),
                        ..
                    })) => {
                        if last_reload.elapsed() > Duration::from_secs(1) {
                            info!("AGENTS.md modified, triggering hot-reload");

                            let agent_metadata_clone = agent_metadata.clone();
                            let mcp_registry_clone = mcp_registry.clone();

                            #[allow(clippy::disallowed_methods)]
                            rt.block_on(async move {
                                let config_content = if std::path::Path::new(".arkavo/AGENTS.md").exists() {
                                    tokio::fs::read_to_string(".arkavo/AGENTS.md").await
                                } else {
                                    tokio::fs::read_to_string("AGENTS.md").await
                                };

                                match config_content {
                                    Ok(content) => {
                                        match reload_configuration_for_watcher(
                                            &content,
                                            agent_metadata_clone,
                                            mcp_registry_clone,
                                        ).await {
                                            Ok(_) => info!("Configuration hot-reload completed successfully"),
                                            Err(e) => {
                                                error!("Configuration hot-reload failed: {}", e);
                                                error!("Agent will continue with existing configuration");
                                            }
                                        }
                                    }
                                    Err(e) => error!("Failed to read AGENTS.md for hot-reload: {}", e),
                                }
                            });
                            last_reload = std::time::Instant::now();
                        }
                    }
                    Ok(Err(e)) => warn!("File watcher error: {}", e),
                    Err(_) => {}
                    _ => {}
                }
            }
        });

        *self.file_watcher_handle.write().await = Some(handle);
        Ok(())
    }

    pub async fn stop_file_watcher(&self) {
        if let Some(handle) = self.file_watcher_handle.write().await.take() {
            handle.abort();
            info!("File watcher stopped");
        }
    }

    pub async fn start_notification_handler(
        &self,
        system_prompt: String,
    ) -> Option<tokio::task::JoinHandle<()>> {
        use std::sync::atomic::Ordering;

        let router_guard = self.router.read().await;
        let router = match router_guard.clone() {
            Some(r) => r,
            None => {
                eprintln!("[Notifications] Cannot start: no router configured");
                return None;
            }
        };
        drop(router_guard);

        let mcp_registry = self.mcp_registry.clone();
        let conductor = self.conductor.read().await.clone();
        let mut notification_rx = mcp_registry.subscribe_notifications();

        let planning_completed = self.planning_completed.clone();
        let agent_plan = self.agent_plan.clone();
        let agent_memory = self.agent_memory.clone();
        let learning_bus = self.learning_bus.read().await.clone();
        let model_hint = self.resolve_model_hint().await;
        #[cfg(feature = "iroh")]
        let notification_iroh_node = self.iroh_node.read().await.clone();

        if std::env::var("ARKAVO_DEBUG").is_ok() {
            eprintln!("[Notifications] Starting push-based notification handler");
        }

        let handle = tokio::spawn(async move {
            loop {
                match notification_rx.recv().await {
                    Ok(notification) => {
                        if std::env::var("ARKAVO_DEBUG").is_ok() {
                            eprintln!(
                                "[Notifications] Received: server={} method={}",
                                notification.server, notification.method
                            );
                        }

                        let event_str = serde_json::to_string(&notification.params)
                            .unwrap_or_else(|_| "{}".to_string());

                        if event_str.contains("\"result\":{}")
                            || event_str.contains("\"content\":[]")
                            || event_str.contains("\"isError\":true")
                        {
                            continue;
                        }

                        let tools = mcp_registry.list_all_tools().await.unwrap_or_default();
                        let should_plan = !planning_completed.load(Ordering::SeqCst)
                            && !system_prompt.is_empty()
                            && !tools.is_empty();

                        if should_plan {
                            eprintln!("[Planning] Conditions met - starting startup planning");
                            planning_completed.store(true, Ordering::SeqCst);

                            let plan = run_startup_planning_phase(
                                &system_prompt,
                                &router,
                                &mcp_registry,
                                &conductor,
                            )
                            .await;

                            *agent_plan.write().await = plan;
                            eprintln!("[Planning] Agent startup planning complete");
                        }

                        let plan = agent_plan.read().await;
                        let goals_section = if !plan.goals.is_empty() {
                            let goals_str: Vec<String> = plan
                                .goals
                                .iter()
                                .enumerate()
                                .map(|(i, g)| {
                                    format!("{}. {} ({:?})", i + 1, g.description, g.status)
                                })
                                .collect();
                            format!("\n\n## Active Goals\n{}", goals_str.join("\n"))
                        } else {
                            String::new()
                        };
                        drop(plan);

                        let memory = agent_memory.read().await;
                        let memory_section = memory.format_control_signals().unwrap_or_default();
                        drop(memory);

                        // Purpose goes as System message; event/goals/memory as User
                        let prompt = format!(
                            "{goals_section}{memory_section}\n\n\
                             ## Event\nServer: {}\nData: {}\n\n\
                             ## Instructions\nConsider your active goals and recent actions when responding. Use tools to take action.",
                            notification.server, event_str
                        );

                        eprintln!(
                            "[Notifications] Processing with LLM: {} chars",
                            prompt.len()
                        );
                        let start_time = std::time::Instant::now();
                        match super::conductor::execute_with_conductor_and_learning(
                            &conductor,
                            &router,
                            &mcp_registry,
                            prompt,
                            None,
                            None,
                            None,
                            None,
                            Some(&system_prompt),
                            None,
                            model_hint.as_ref(),
                            None,
                            None,
                            None,
                            #[cfg(feature = "iroh")]
                            notification_iroh_node.as_ref(),
                        )
                        .await
                        {
                            Ok(result) => {
                                let latency_ms = start_time.elapsed().as_millis() as u64;
                                eprintln!("[Notifications] LLM result: {} chars", result.len());
                                if !result.is_empty() {
                                    info!("Notification processed: {} chars", result.len());
                                    debug!("Notification result: {}", result);
                                }

                                // Emit learning event for successful tool execution
                                if let Some(bus) = &learning_bus {
                                    let event = super::learning_bus::LearningEvent::ToolCall {
                                        tool_name: notification.method.clone(),
                                        args: notification.params.clone().unwrap_or_default(),
                                        result: result.clone(),
                                        success: true,
                                        latency_ms,
                                        decision_trace_id: None,
                                        step_index: 0,
                                        model_name: None,
                                    };
                                    let _ = bus.sender().send(event).await;
                                }
                            }
                            Err(e) => {
                                let latency_ms = start_time.elapsed().as_millis() as u64;
                                eprintln!("[Notifications] Processing failed: {e}");
                                warn!("Notification processing failed: {}", e);

                                // Emit learning event for failed tool execution
                                if let Some(bus) = &learning_bus {
                                    let event = super::learning_bus::LearningEvent::ToolCall {
                                        tool_name: notification.method.clone(),
                                        args: notification.params.clone().unwrap_or_default(),
                                        result: format!("Error: {e}"),
                                        success: false,
                                        latency_ms,
                                        decision_trace_id: None,
                                        step_index: 0,
                                        model_name: None,
                                    };
                                    let _ = bus.sender().send(event).await;
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Notification handler lagged, missed {} messages", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("Notification channel closed, stopping handler");
                        break;
                    }
                }
            }
        });

        Some(handle)
    }

    /// Rebuild tool registry from MCP connections
    ///
    /// Fixes race condition where tools are queried before MCP servers connect.
    /// Called just before server start to ensure all MCP tools are registered.
    async fn rebuild_tool_registry(&self) {
        use super::mcp_bridge::McpBridgeTool;

        let mcp_tools = match self.mcp_registry.list_all_tools().await {
            Ok(tools) => tools,
            Err(e) => {
                warn!("Failed to list MCP tools for rebuild: {}", e);
                return;
            }
        };

        if mcp_tools.is_empty() {
            debug!("No MCP tools to register");
            return;
        }

        info!(
            "Rebuilding tool registry with {} MCP tools",
            mcp_tools.len()
        );

        let mut tool_registry = arkavo_mcp_tools::ToolRegistry::empty();

        for tool in mcp_tools {
            let tool_name = tool.name.clone();
            let bridge = McpBridgeTool::new(self.mcp_registry.clone(), tool);
            tool_registry.register(&tool_name, Box::new(bridge));
        }

        *self.tool_registry.write().await = Some(Arc::new(tool_registry));
    }

    /// Start the server and return the handle along with the actual bound port
    pub async fn start(&self) -> Result<ServerHandle> {
        let (handle, _actual_port) = self.start_with_port().await?;
        Ok(handle)
    }

    /// Start the server and return both the handle and the actual bound port
    /// This is useful when binding to port 0 for dynamic port allocation
    pub async fn start_with_port(&self) -> Result<(ServerHandle, u16)> {
        let addr: SocketAddr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| A2aError::InvalidEndpoint(format!("Invalid bind address: {e}")))?;

        info!("Starting A2A server on {}", addr);

        // Rebuild tool registry now that MCP servers have been registered
        self.rebuild_tool_registry().await;

        let server_cfg = jsonrpsee::server::ServerConfig::builder()
            .max_connections(self.config.max_connections as u32)
            .build();
        let proxy_layer = ProxyGetRequestLayer::new([
            ("/.well-known/agent.json", "agent_card"),
            ("/health", "health"),
        ])
        .map_err(|e| A2aError::Transport(format!("Failed to create proxy layer: {e}")))?;
        let server = ServerBuilder::with_config(server_cfg)
            .set_http_middleware(tower::ServiceBuilder::new().layer(proxy_layer))
            .build(addr)
            .await
            .map_err(|e| A2aError::Transport(format!("Failed to build server: {e}")))?;

        // Get the actual bound address (important when using port 0)
        let actual_addr = server
            .local_addr()
            .map_err(|e| A2aError::Transport(format!("Failed to get local address: {e}")))?;
        let actual_port = actual_addr.port();

        if self.config.port == 0 {
            info!("Server bound to dynamic port: {}", actual_port);
        }

        let rate_limiter = Arc::new(RateLimiter::new(self.config.rate_limit.clone()));
        let metrics = Arc::new(MetricsCollector::new(self.config.metrics_enabled));
        let llm_adapter = self.llm_adapter.read().await.clone();
        let router = self.router.read().await.clone();
        let tool_registry = self.tool_registry.read().await.clone();

        // Create shared learning context for chat — updated from LearningBus
        let learning_context: Arc<tokio::sync::RwLock<String>> =
            Arc::new(tokio::sync::RwLock::new(String::new()));
        if let Some(bus) = self.learning_bus().await {
            let lc = learning_context.clone();
            tokio::spawn(async move {
                loop {
                    let guidance = bus.get_behavior_guidance(None).await;
                    let trends = bus.get_quality_trends().await;
                    let lesson_count = bus.behavior_lesson_count().await;

                    let mut ctx = String::new();
                    if !guidance.is_empty() {
                        ctx.push_str("Lessons learned:\n");
                        ctx.push_str(&guidance);
                        ctx.push('\n');
                    }
                    if !trends.is_empty() {
                        use std::fmt::Write;
                        let _ = write!(ctx, "Quality trends: {} tracked. ", trends.len());
                    }
                    if lesson_count > 0 {
                        use std::fmt::Write;
                        let _ = write!(ctx, "Total lessons: {lesson_count}. ");
                    }
                    // Cap at 512 tokens (~2048 chars)
                    ctx.truncate(2048);
                    *lc.write().await = ctx;
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            });
        }

        // Create shared task context for chat — updated from ToolMemory (recent actions + last observed state)
        let task_context: Arc<tokio::sync::RwLock<String>> =
            Arc::new(tokio::sync::RwLock::new(String::new()));
        {
            let tc = task_context.clone();
            let agent_memory = self.agent_memory.clone();
            tokio::spawn(async move {
                loop {
                    let memory = agent_memory.read().await;
                    let mut ctx = String::new();

                    // Include last observed state (environment state)
                    if let Some(state) = memory.last_observe_full() {
                        use std::fmt::Write;
                        let _ = write!(ctx, "## Current Observed State\n{state}\n");
                    }

                    // Include control signals (dedup, errors, setup state)
                    if let Some(signals) = memory.format_control_signals() {
                        ctx.push_str(&signals);
                    }

                    drop(memory);

                    // Cap to avoid exceeding context limits (floor to char boundary)
                    if ctx.len() > 4096 {
                        let mut end = 4096;
                        while !ctx.is_char_boundary(end) {
                            end -= 1;
                        }
                        ctx.truncate(end);
                    }
                    *tc.write().await = ctx;
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            });
        }

        let chat_sessions = if let Some(router_instance) = router.clone() {
            info!(
                "✓ ChatSessionManager will be created WITH Router (dynamic model selection + quality gates + tools)"
            );
            let mut mgr = chat_session::ChatSessionManager::with_config(
                None,
                Some(router_instance),
                tool_registry.clone(),
                3600,
                self.buffer_config.clone(),
            );
            mgr.set_learning_context(learning_context.clone());
            mgr.set_task_context(task_context.clone());

            // Inject agent purpose from AGENTS.md as system prompt for chat context
            {
                let metadata = self.agent_metadata.read().await;
                mgr.set_system_prompt(metadata.purpose.clone());
            }

            // Wire teaching intent from chat → learning bus
            if let Some(bus) = self.learning_bus().await {
                let (teaching_tx, mut teaching_rx) =
                    tokio::sync::mpsc::channel::<chat_session::ChatTeachingEvent>(64);
                mgr.set_teaching_tx(teaching_tx);
                tokio::spawn(async move {
                    while let Some(evt) = teaching_rx.recv().await {
                        match evt {
                            chat_session::ChatTeachingEvent::Instruction { text, scope } => {
                                bus.inject_human_instruction(&text, scope, "chat").await;
                            }
                            chat_session::ChatTeachingEvent::Correction { text, trace_id } => {
                                bus.record_human_correction(&text, trace_id, None).await;
                            }
                            chat_session::ChatTeachingEvent::Reinforcement { trace_id } => {
                                bus.record_human_reinforcement(trace_id).await;
                            }
                        }
                    }
                });
                info!("✓ Human teaching wired: chat → learning bus");
            }

            Arc::new(mgr)
        } else if llm_adapter.is_some() {
            info!("✓ ChatSessionManager will be created WITH LLM adapter");
            Arc::new(chat_session::ChatSessionManager::with_config(
                llm_adapter.clone(),
                None,
                None,
                3600,
                self.buffer_config.clone(),
            ))
        } else {
            warn!(
                "✗ ChatSessionManager will be created WITHOUT LLM adapter or router - messages will fail!"
            );
            Arc::new(chat_session::ChatSessionManager::with_config(
                None,
                None,
                None,
                3600,
                self.buffer_config.clone(),
            ))
        };

        let task_store: Arc<dyn TaskStore> =
            match &self.config.task_store_path {
                Some(path) => {
                    let task_store_path = std::path::Path::new(path);
                    Arc::new(SqliteTaskStore::new(task_store_path).await.map_err(|e| {
                        A2aError::Internal(format!("Failed to create task store: {e}"))
                    })?)
                }
                None => Arc::new(SqliteTaskStore::new_in_memory().await.map_err(|e| {
                    A2aError::Internal(format!("Failed to create in-memory task store: {e}"))
                })?),
            };

        let task_executor = Arc::new(TaskExecutor::with_metrics(
            task_store.clone(),
            TaskExecutorConfig::default(),
            metrics.clone(),
        ));

        task_executor
            .start()
            .map_err(|e| A2aError::Internal(format!("Failed to start task executor: {e}")))?;

        // Start shared Iroh P2P node for TDF blob transport
        #[cfg(feature = "iroh")]
        {
            match arkavo_tdf_iroh::IrohNode::memory().await {
                Ok(node) => {
                    info!("Iroh P2P node started for TDF blob transport");
                    *self.iroh_node.write().await = Some(node);
                }
                Err(e) => {
                    warn!("Failed to start Iroh P2P node: {e}");
                }
            }
        }

        let rpc_impl = A2aRpcImpl {
            rate_limiter,
            metrics,
            mcp_registry: self.mcp_registry.clone(),
            agent_metadata: self.agent_metadata.clone(),
            llm_adapter,
            chat_sessions,
            task_store,
            task_executor,
            event_writer: self.event_writer.read().await.clone(),
            session_id: self.session_id.clone(),
            event_sequence: self.event_sequence.clone(),
            auth_backend: Arc::new(arkavo_protocol::auth::JwtAuthBackend::new(
                "change-me-in-production",
            )),
            registration_service: Arc::new(arkavo_agent::registration::RegistrationService::new()),
            conductor: self.conductor.read().await.clone(),
            router,
            learning_bus: self.learning_bus.read().await.clone(),
            public_key: self.public_key.read().await.clone(),
            budget_manager: self.budget_manager.read().await.clone(),
            orchestrator_tick: self.orchestrator_tick.clone(),
            compute_budget: self.compute_budget.clone(),
            mesh_state: Some(self.mesh_state.clone()),
            agent_memory: self.agent_memory.clone(),
            model_hint: self.resolve_model_hint().await,
            #[cfg(feature = "kas")]
            kas_handler: {
                let agent_config = self.agent_config.read().await;
                let kas_config = agent_config.kas.clone().map(|k| arkavo_tdf::KasA2aConfig {
                    key_id: k.key_id.unwrap_or_else(|| "kas-key-1".to_string()),
                    algorithm: k.algorithm.unwrap_or_else(|| "ec:secp256r1".to_string()),
                });
                let mut handler = KasA2aHandler::new(vec![], kas_config.unwrap_or_default());
                handler.set_keypair(arkavo_tdf::KasKeypair::generate());
                Some(Arc::new(handler))
            },
            #[cfg(feature = "kas")]
            tdf_offer_store: Arc::new(super::handlers::tdf_share::TdfOfferStore::new()),
            agent_event_tx: self.agent_event_tx.clone(),
            #[cfg(feature = "iroh")]
            iroh_node: self.iroh_node.read().await.clone(),
        };

        if let Err(e) = self.start_file_watcher().await {
            warn!("Failed to start file watcher: {}", e);
        }

        let handle = server.start(rpc_impl.into_rpc());

        info!("A2A server started successfully on {}", actual_addr);
        info!(
            "Agent Card available at http://{}:{}/.well-known/agent.json",
            self.config.bind_address, actual_port
        );

        Ok((handle, actual_port))
    }

    /// Start the autonomous orchestrator loop.
    ///
    /// Called by the agent CLI after startup when the agent has MCP tools configured
    /// (i.e., it's an orchestrator). Specialists don't call this.
    ///
    /// Loop: observe → plan → delegate → execute → sleep → repeat
    pub async fn start_orchestrator_loop(&self) {
        let router_guard = self.router.read().await;
        let router = match router_guard.clone() {
            Some(r) => r,
            None => {
                warn!("Cannot start orchestrator loop: no router configured");
                return;
            }
        };
        drop(router_guard);

        let conductor = self.conductor.read().await.clone();
        let mcp_registry = self.mcp_registry.clone();
        let agent_memory = self.agent_memory.clone();
        let learning_bus = self.learning_bus.read().await.clone();
        let mesh_state = self.mesh_state.clone();
        let compute_budget = self.compute_budget.clone();
        let model_hint = self.resolve_model_hint().await;
        let orchestrator_tick = self.orchestrator_tick.clone();
        let has_mcp_tools = self
            .mcp_registry
            .list_all_tools()
            .await
            .map(|tools| !tools.is_empty())
            .unwrap_or(false);
        let total_ram_bytes = self.total_ram_bytes;
        let metadata = self.agent_metadata.read().await;
        let self_agent_id = metadata.name.clone();
        let commander_model = metadata.model.clone();
        let purpose = metadata.purpose.clone();
        let agent_mode = metadata.mode.clone();
        drop(metadata);

        #[cfg(feature = "iroh")]
        let iroh_node = self.iroh_node.read().await.clone();

        info!(
            mode = ?agent_mode,
            "Starting agent loop"
        );

        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<super::agent_event::AgentEvent>(32);

        // Store the event sender so the message handler can inject A2A messages
        *self.agent_event_tx.lock().await = Some(event_tx);

        let config = super::agent_loop::AgentLoopConfig {
            conductor,
            router,
            mcp_registry,
            agent_memory,
            learning_bus,
            mesh_state,
            compute_budget,
            model_hint,
            purpose,
            orchestrator_tick,
            has_mcp_tools,
            tool_loop_budget: None,
            total_ram_bytes,
            self_agent_id,
            commander_model,
            agent_mode,
            #[cfg(feature = "iroh")]
            iroh_node,
        };

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            config.mesh_state.discover_peers().await;
            super::agent_loop::run_agent_loop(config, event_rx).await;
        });
    }

    /// Release GPU resources for graceful shutdown.
    ///
    /// Must be called before std::process::exit() to ensure Metal
    /// residency sets are properly cleaned up.
    ///
    /// Uses timeouts to prevent deadlocks if locks are held during shutdown.
    pub async fn cleanup_gpu_resources(&self) {
        use tokio::time::{Duration, timeout};

        info!("Releasing GPU resources for graceful shutdown");

        const TIMEOUT_MS: u64 = 5000; // 5 second timeout for each lock
        let timeout_duration = Duration::from_millis(TIMEOUT_MS);

        // Clear router (holds TaskClassifier -> LlamaCppProvider -> LlamaModel)
        match timeout(timeout_duration, self.router.write()).await {
            Ok(mut guard) => {
                *guard = None;
                info!("Router cleared");
            }
            Err(_) => {
                warn!(
                    "Timeout acquiring router write lock during cleanup (lock held > {}ms)",
                    TIMEOUT_MS
                );
            }
        }

        // Clear LLM adapter (may hold model references)
        match timeout(timeout_duration, self.llm_adapter.write()).await {
            Ok(mut guard) => {
                *guard = None;
                info!("LLM adapter cleared");
            }
            Err(_) => {
                warn!(
                    "Timeout acquiring LLM adapter write lock during cleanup (lock held > {}ms)",
                    TIMEOUT_MS
                );
            }
        }

        info!("GPU resources released");

        // Stop Iroh P2P node
        #[cfg(feature = "iroh")]
        {
            match timeout(timeout_duration, self.iroh_node.write()).await {
                Ok(mut guard) => {
                    if let Some(node) = guard.take() {
                        if let Err(e) = node.stop().await {
                            warn!("Failed to stop Iroh node: {e}");
                        } else {
                            info!("Iroh P2P node stopped");
                        }
                    }
                }
                Err(_) => {
                    warn!(
                        "Timeout acquiring Iroh node write lock during cleanup (lock held > {}ms)",
                        TIMEOUT_MS
                    );
                }
            }
        }
    }
}

/// Convert AGENTS.md budget YAML to BudgetConfig for the budget manager
fn build_budget_config(yaml: &arkavo_router::BudgetYamlConfig) -> arkavo_budget::BudgetConfig {
    let mut config = arkavo_budget::BudgetConfig::default();

    if let Some(session_cost) = yaml.max_cost_per_session {
        config.limits.session_limit = Some(arkavo_budget::TokenCost::from_dollars(session_cost));
    }
    if let Some(daily_cost) = yaml.max_cost_per_day {
        config.limits.daily_limit = Some(arkavo_budget::TokenCost::from_dollars(daily_cost));
    }

    config
}

#[cfg(test)]
mod dedup_tests {
    use arkavo_test_macros::spec;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    /// Hash context components the same way the orchestrator does:
    /// actions + specialist context + errors + dead-man category (not cycle number)
    fn context_hash(actions: &str, specialist: &str, errors: &str, no_action_ticks: u32) -> u64 {
        let mut hasher = DefaultHasher::new();
        actions.hash(&mut hasher);
        specialist.hash(&mut hasher);
        errors.hash(&mut hasher);
        no_action_ticks.min(5).hash(&mut hasher);
        hasher.finish()
    }

    #[spec("SRV-009")]
    #[test]
    fn test_same_context_different_ticks_same_hash() {
        let h1 = context_hash("action-a: OK", "", "", 3);
        let h2 = context_hash("action-a: OK", "", "", 3);
        assert_eq!(h1, h2, "identical context must produce identical hash");
    }

    #[spec("SRV-009")]
    #[test]
    fn test_different_actions_different_hash() {
        let h1 = context_hash("action-a: OK", "", "", 3);
        let h2 = context_hash("action-b: started", "", "", 3);
        assert_ne!(h1, h2);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_specialist_response_changes_hash() {
        let h1 = context_hash("action-a: OK", "", "", 5);
        let h2 = context_hash("action-a: OK", "peer-1: new recommendation", "", 5);
        assert_ne!(h1, h2, "new specialist advice must change hash");
    }

    #[spec("SRV-009")]
    #[test]
    fn test_dead_man_category_caps_at_5() {
        // Ticks 5, 6, 7... all hash the same (capped at 5)
        let h5 = context_hash("", "", "", 5);
        let h6 = context_hash("", "", "", 6);
        let h99 = context_hash("", "", "", 99);
        assert_eq!(h5, h6);
        assert_eq!(h5, h99);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_dedup_logic_skips_when_stuck() {
        let mut last_hash: u64 = 0;
        let mut consecutive_dupes: u32 = 0;
        let consecutive_no_action: u32 = 3;
        let mut executed = Vec::new();

        for cycle in 1..=10 {
            let hash = context_hash("action-a: status OK", "", "", consecutive_no_action);

            if hash == last_hash && consecutive_no_action >= 2 {
                consecutive_dupes += 1;
                if consecutive_dupes % 5 != 0 {
                    continue;
                }
            } else {
                consecutive_dupes = 0;
            }
            last_hash = hash;
            executed.push(cycle);
        }

        // Cycle 1 executes (first, no previous hash)
        // Cycles 2-5 skipped (duplicate context, no action)
        // Cycle 6 executes (every 5th allowed through)
        assert_eq!(executed, vec![1, 6]);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_dedup_resets_on_new_specialist_advice() {
        let mut last_hash: u64 = 0;
        let mut consecutive_dupes: u32 = 0;
        let consecutive_no_action: u32 = 5;
        let mut executed = Vec::new();

        let contexts: Vec<(&str, &str)> = vec![
            ("action-a: OK", ""),
            ("action-a: OK", ""),
            ("action-a: OK", ""),
            ("action-a: OK", "peer-1: new recommendation"), // new specialist advice
            ("action-a: OK", "peer-1: new recommendation"),
            ("action-a: OK", "peer-1: new recommendation"),
        ];

        for (i, (actions, specialist)) in contexts.iter().enumerate() {
            let hash = context_hash(actions, specialist, "", consecutive_no_action);
            if hash == last_hash && consecutive_no_action >= 2 {
                consecutive_dupes += 1;
                if consecutive_dupes % 5 != 0 {
                    continue;
                }
            } else {
                consecutive_dupes = 0;
            }
            last_hash = hash;
            executed.push(i);
        }

        // cycle 0: first context — executes
        // cycle 1-2: duplicate — skipped
        // cycle 3: new specialist advice — executes (reset)
        // cycle 4-5: duplicate — skipped
        assert_eq!(executed, vec![0, 3]);
    }
}
