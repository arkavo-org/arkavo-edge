use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use arkavo_events::{Event, EventPayload, EventWriter, EventWriterConfig};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_llm::{LlmClient, LlmClientAdapter, LlmConfig};
use jsonrpsee::server::{ServerBuilder, ServerHandle};

use crate::auth::NoOpAuthBackend;
use crate::config::{BufferConfig, ServerConfig};
use crate::error::{A2aError, Result};
use crate::mcp_registry::McpRegistry;
use crate::metrics::MetricsCollector;
use crate::rate_limit::RateLimiter;
use crate::task_executor::{TaskExecutor, TaskExecutorConfig};
use crate::task_store::{SqliteTaskStore, TaskStore};

use super::config_helpers::{AgentMetadata, reload_configuration_for_watcher};
use super::learning_bus::LearningBus;
use super::startup::{AgentPlan, run_startup_planning_phase};
use super::tool_memory::ToolMemory;
use super::well_known::{WellKnownState, start_well_known_server};
use super::{A2aRpcImpl, A2aRpcServer, execute_with_conductor};

#[cfg(feature = "kas")]
use arkavo_tdf::KasA2aHandler;

pub struct A2aServer {
    config: ServerConfig,
    buffer_config: BufferConfig,
    mcp_registry: Arc<McpRegistry>,
    agent_metadata: Arc<tokio::sync::RwLock<AgentMetadata>>,
    llm_adapter: Arc<tokio::sync::RwLock<Option<Arc<LlmClientAdapter>>>>,
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
    /// Handle for the well-known HTTP server
    well_known_handle: Arc<tokio::sync::RwLock<Option<tokio::task::JoinHandle<()>>>>,
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
            well_known_handle: Arc::new(tokio::sync::RwLock::new(None)),
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

    /// Set the learning bus for gossip-based learning
    pub async fn set_learning_bus(&self, bus: Arc<LearningBus>) {
        *self.learning_bus.write().await = Some(bus);
    }

    /// Get the learning bus reference
    pub async fn learning_bus(&self) -> Option<Arc<LearningBus>> {
        self.learning_bus.read().await.clone()
    }

    pub async fn set_agent_metadata(&self, name: String, purpose: String, model: String) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.name.clone_from(&name);
        metadata.purpose = purpose;
        metadata.model.clone_from(&model);
        metadata.endpoint = format!("http://{}:{}", self.config.bind_address, self.config.port);
        drop(metadata);

        // Always initialize router for task execution via HRM conductor
        // Router is required by execute_with_conductor regardless of model
        info!("Initializing router for task execution");
        self.initialize_router().await;
        self.build_tool_registry().await;

        // Also create LLM adapter if model is specified
        if !model.is_empty() {
            self.recreate_llm_adapter().await;
        }
    }

    pub async fn set_api_keys(&self, api_keys: std::collections::HashMap<String, String>) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.api_keys = api_keys;
        drop(metadata);
        self.recreate_llm_adapter().await;
    }

    #[allow(dead_code)]
    async fn reload_configuration_from_content(&self, content: &str) -> Result<()> {
        use crate::agent_config::parse_agents_config;

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

        let model_changed = {
            let metadata = self.agent_metadata.read().await;
            metadata.model != new_config.model
        };

        self.set_agent_metadata(
            new_config.name.clone(),
            new_config.purpose.clone(),
            new_config.model.clone(),
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

    async fn recreate_llm_adapter(&self) {
        let metadata = self.agent_metadata.read().await;
        let model = metadata.model.clone();
        let api_keys = metadata.api_keys.clone();
        drop(metadata);

        info!("Attempting to create LLM adapter for model: {}", model);
        match self.create_llm_adapter(&model, &api_keys) {
            Ok(adapter) => {
                *self.llm_adapter.write().await = Some(adapter);
                info!("✓ Successfully created LLM adapter with model: {}", model);
            }
            Err(e) => {
                error!(model = model, error = %e, "✗ Failed to create LLM adapter");
            }
        }
    }

    pub(super) fn create_llm_adapter(
        &self,
        model_url: &str,
        api_keys: &std::collections::HashMap<String, String>,
    ) -> Result<Arc<LlmClientAdapter>> {
        if let Some((provider, rest)) = model_url.split_once("://") {
            match provider {
                "ollama" => {
                    if let Some((host_port, model_name)) = rest.rsplit_once('/') {
                        let config =
                            LlmConfig::ollama_with(format!("http://{host_port}"), model_name);
                        let client = LlmClient::from_config(&config).map_err(|e| {
                            A2aError::InvalidRequest(format!("Failed to create LLM client: {e}"))
                        })?;
                        Ok(Arc::new(LlmClientAdapter::new(client)))
                    } else {
                        Err(A2aError::InvalidRequest(format!(
                            "Invalid Ollama URL format: {model_url}"
                        )))
                    }
                }
                "kimi" => {
                    let api_key = api_keys
                        .get("MOONSHOT_API_KEY")
                        .cloned()
                        .or_else(|| std::env::var("MOONSHOT_API_KEY").ok());

                    let mut config = if let Some(key) = api_key {
                        LlmConfig::kimi(key)
                    } else {
                        return Err(A2aError::InvalidRequest(
                            "MOONSHOT_API_KEY not provided in config or environment".to_string(),
                        ));
                    };

                    if !rest.is_empty() {
                        config.model = Some(rest.to_string());
                    }

                    let client = LlmClient::from_config(&config).map_err(|e| {
                        A2aError::InvalidRequest(format!("Failed to create KIMI client: {e}"))
                    })?;
                    Ok(Arc::new(LlmClientAdapter::new(client)))
                }
                _ => Err(A2aError::InvalidRequest(format!(
                    "Unsupported LLM provider: {provider}"
                ))),
            }
        } else {
            info!(
                "Model '{}' has no provider prefix, attempting to create from environment variables",
                model_url
            );

            let config = LlmConfig::from_env();
            let client = LlmClient::from_config(&config).map_err(|e| {
                A2aError::InvalidRequest(format!(
                    "Failed to create LLM client for model '{model_url}' from environment: {e}. \
                     Either use format 'provider://host:port/model' or set LLM_PROVIDER env var"
                ))
            })?;

            info!(
                "Successfully created LLM client from environment for model: {}",
                model_url
            );
            Ok(Arc::new(LlmClientAdapter::new(client)))
        }
    }

    async fn initialize_router(&self) {
        // Check for offline mode (local models only)
        let offline_mode = std::env::var("ARKAVO_OFFLINE").is_ok();
        if offline_mode {
            info!("Initializing router in OFFLINE mode (local models only)");
        } else {
            info!("Initializing router for dynamic model selection");
        }

        let router_result = if offline_mode {
            arkavo_router::Router::new_offline().await
        } else {
            arkavo_router::Router::new().await
        };

        match router_result {
            Ok(router) => {
                let router = Arc::new(router);
                *self.router.write().await = Some(router.clone());
                info!(
                    "✓ Successfully initialized router (offline={})",
                    offline_mode
                );

                // Set router on learning bus for LLM-based synthesis
                if let Some(bus) = self.learning_bus.read().await.as_ref() {
                    bus.set_router(router.clone()).await;
                    info!("✓ Router configured for learning synthesis");
                }
            }
            Err(e) => {
                error!(error = %e, "✗ Failed to initialize router");
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
                    // Tools from MCP servers will be registered via the McpRegistry
                    // The registry serves as a unified view for the router
                    info!("  - {} (from MCP)", tool.name);
                }
            }
            Err(e) => {
                warn!("Failed to list tools from MCP servers: {}", e);
            }
        }

        // Also register the list_models tool from the router for model discovery
        if let Some(router) = self.router.read().await.as_ref() {
            arkavo_router::tools::register_tools(&mut tool_registry, router.clone());
        }

        *self.tool_registry.write().await = Some(Arc::new(tool_registry));
        info!("✓ Tool registry built (MCP tools only)");
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
        let llm_adapter = self.llm_adapter.clone();
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
                            let llm_adapter_clone = llm_adapter.clone();
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
                                            llm_adapter_clone,
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
                        let memory_section = memory.format_for_prompt();
                        drop(memory);

                        let prompt = format!(
                            "{}{}{}\n\n## Event\nServer: {}\nData: {}\n\n## Instructions\nConsider your active goals and recent actions when responding. Use tools to take action.",
                            system_prompt,
                            goals_section,
                            memory_section,
                            notification.server,
                            event_str
                        );

                        eprintln!(
                            "[Notifications] Processing with LLM: {} chars",
                            prompt.len()
                        );
                        let start_time = std::time::Instant::now();
                        match execute_with_conductor(
                            &conductor,
                            &router,
                            &mcp_registry,
                            prompt,
                            None,
                            None,
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

        let server = ServerBuilder::default()
            .max_connections(self.config.max_connections as u32)
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

        let chat_sessions = if let Some(router_instance) = router.clone() {
            info!(
                "✓ ChatSessionManager will be created WITH Router (dynamic model selection + quality gates + tools)"
            );
            Arc::new(crate::chat_session::ChatSessionManager::with_config(
                None,
                Some(router_instance),
                tool_registry.clone(),
                3600,
                self.buffer_config.clone(),
            ))
        } else if llm_adapter.is_some() {
            info!("✓ ChatSessionManager will be created WITH LLM adapter");
            Arc::new(crate::chat_session::ChatSessionManager::with_config(
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
            Arc::new(crate::chat_session::ChatSessionManager::with_config(
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
            auth_backend: Arc::new(NoOpAuthBackend),
            registration_service: Arc::new(crate::registration::RegistrationService::new()),
            conductor: self.conductor.read().await.clone(),
            router,
            learning_bus: self.learning_bus.read().await.clone(),
            public_key: self.public_key.read().await.clone(),
            #[cfg(feature = "kas")]
            kas_handler: Some(Arc::new(KasA2aHandler::with_defaults())),
        };

        if let Err(e) = self.start_file_watcher().await {
            warn!("Failed to start file watcher: {}", e);
        }

        let handle = server.start(rpc_impl.into_rpc());

        info!("A2A server started successfully on {}", actual_addr);
        info!("OpenRPC schema available via JSON-RPC method: rpc.discover");

        // Start the well-known HTTP server on port + 1 (or dynamic if port is 0)
        let http_port = if self.config.port == 0 {
            0
        } else {
            self.config.port + 1
        };

        if let Err(e) = self.start_well_known_server(http_port, actual_port).await {
            warn!("Failed to start well-known HTTP server: {}", e);
        }

        Ok((handle, actual_port))
    }

    /// Start the well-known HTTP server for agent discovery
    /// Serves /.well-known/agent.json per A2A protocol spec
    pub async fn start_well_known_server(&self, http_port: u16, rpc_port: u16) -> Result<u16> {
        let bind_addr: SocketAddr = format!("{}:{}", self.config.bind_address, http_port)
            .parse()
            .map_err(|e| A2aError::InvalidEndpoint(format!("Invalid HTTP bind address: {e}")))?;

        let state = WellKnownState {
            agent_metadata: self.agent_metadata.clone(),
            mcp_registry: self.mcp_registry.clone(),
            rpc_port,
            #[cfg(feature = "kas")]
            kas_enabled: true,
        };

        let (handle, actual_http_port) = start_well_known_server(bind_addr, state)
            .await
            .map_err(|e| A2aError::Transport(format!("Failed to start well-known server: {e}")))?;

        *self.well_known_handle.write().await = Some(handle);

        info!(
            "Agent Card available at http://{}:{}/.well-known/agent.json",
            self.config.bind_address, actual_http_port
        );

        Ok(actual_http_port)
    }

    /// Stop the well-known HTTP server
    pub async fn stop_well_known_server(&self) {
        if let Some(handle) = self.well_known_handle.write().await.take() {
            handle.abort();
            info!("Well-known HTTP server stopped");
        }
    }

    /// Release GPU resources for graceful shutdown.
    ///
    /// Must be called before std::process::exit() to ensure Metal
    /// residency sets are properly cleaned up.
    pub async fn cleanup_gpu_resources(&self) {
        info!("Releasing GPU resources for graceful shutdown");

        // Clear router (holds TaskClassifier -> LlamaCppProvider -> LlamaModel)
        *self.router.write().await = None;

        // Clear LLM adapter (may hold model references)
        *self.llm_adapter.write().await = None;

        info!("GPU resources released");
    }
}
