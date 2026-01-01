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

use super::conductor::execute_with_conductor_and_model;
use super::config_helpers::{AgentMetadata, reload_configuration_for_watcher};
use super::learning_bus::LearningBus;
use super::startup::{AgentPlan, run_startup_planning_phase};
use super::tool_memory::ToolMemory;
use super::{A2aRpcImpl, A2aRpcServer};

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
    /// Shared conversation context for the agent - all chat sessions share this
    agent_context: Arc<tokio::sync::RwLock<Vec<arkavo_llm::Message>>>,
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
            agent_context: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
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

    /// Get the shared agent context
    pub fn agent_context(&self) -> Arc<tokio::sync::RwLock<Vec<arkavo_llm::Message>>> {
        self.agent_context.clone()
    }

    /// Append a message to the agent's shared context
    pub async fn append_to_context(&self, message: arkavo_llm::Message) {
        self.agent_context.write().await.push(message);
    }

    /// Append multiple messages to the agent's shared context
    pub async fn extend_context(&self, messages: Vec<arkavo_llm::Message>) {
        self.agent_context.write().await.extend(messages);
    }

    pub async fn set_agent_metadata(
        &self,
        name: String,
        purpose: String,
        model: String,
        action_interval: u64,
    ) {
        let mut metadata = self.agent_metadata.write().await;
        metadata.name.clone_from(&name);
        metadata.purpose = purpose;
        metadata.model.clone_from(&model);
        metadata.endpoint = format!("http://{}:{}", self.config.bind_address, self.config.port);
        metadata.action_interval = action_interval;
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

        // Note: action_interval not yet supported in hot-reload config
        let current_interval = self.agent_metadata.read().await.action_interval;
        self.set_agent_metadata(
            new_config.name.clone(),
            new_config.purpose.clone(),
            new_config.model.clone(),
            current_interval, // Preserve existing interval
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
        info!("Initializing router for dynamic model selection");
        match arkavo_router::Router::new().await {
            Ok(router) => {
                let router = Arc::new(router);
                *self.router.write().await = Some(router.clone());
                info!("✓ Successfully initialized router");

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
        use crate::server::mcp_bridge::McpBridgeTool;

        info!("Building tool registry from MCP connections (progressive discovery mode)");

        // Use empty registry - agents discover tools progressively via MCP servers only
        // No default GitHub/Git/TDF/browser tools - clean slate for specialized agents
        let mut tool_registry = arkavo_mcp_tools::ToolRegistry::empty();

        match self.mcp_registry.list_all_tools().await {
            Ok(tools) => {
                info!("Found {} tools from MCP servers", tools.len());
                // Register MCP tools as bridges
                for tool in tools {
                    let tool_name = tool.name.clone();
                    let bridge = McpBridgeTool::new(self.mcp_registry.clone(), tool);
                    tool_registry.register(&tool_name, Box::new(bridge));
                }
            }
            Err(e) => {
                warn!("Failed to list tools from MCP servers: {}", e);
            }
        }

        *self.tool_registry.write().await = Some(Arc::new(tool_registry));
        info!("✓ Tool registry built successfully");
    }

    /// Rebuild the tool registry with current MCP tools.
    /// Called at server start time after MCP servers have been registered.
    async fn rebuild_tool_registry(&self) {
        use crate::server::mcp_bridge::McpBridgeTool;

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

        // Use empty registry for progressive tool discovery
        // Small models can't handle 30+ tools - they discover via REQUEST_TOOL: protocol
        let mut tool_registry = arkavo_mcp_tools::ToolRegistry::empty();

        // Register each MCP tool as a bridge
        for tool in mcp_tools {
            let tool_name = tool.name.clone();
            let bridge = McpBridgeTool::new(self.mcp_registry.clone(), tool);
            tool_registry.register(&tool_name, Box::new(bridge));
        }

        let tool_names: Vec<_> = tool_registry
            .list_tools()
            .iter()
            .map(|t| t.name.clone())
            .collect();
        info!("✓ Tool registry rebuilt with tools: {:?}", tool_names);

        *self.tool_registry.write().await = Some(Arc::new(tool_registry));
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
        let agent_context = self.agent_context.clone();
        let agent_metadata = self.agent_metadata.clone();

        eprintln!("[Notifications] Starting push-based notification handler");

        let handle = tokio::spawn(async move {
            // Run startup planning immediately using the purpose as initial prompt
            if !system_prompt.is_empty() {
                // Poll for MCP tools with exponential backoff (max 30 seconds)
                let mut tools = Vec::new();
                let mut wait_ms = 500;
                let max_wait_total = 30_000;
                let mut total_waited = 0;

                while tools.is_empty() && total_waited < max_wait_total {
                    tokio::time::sleep(tokio::time::Duration::from_millis(wait_ms)).await;
                    total_waited += wait_ms;
                    tools = mcp_registry.list_all_tools().await.unwrap_or_default();
                    if tools.is_empty() {
                        eprintln!(
                            "[Planning] Waiting for MCP tools... ({}/{}ms)",
                            total_waited, max_wait_total
                        );
                        wait_ms = (wait_ms * 2).min(5000); // exponential backoff, max 5s
                    }
                }

                if tools.is_empty() {
                    eprintln!(
                        "[Planning] No MCP tools registered after {}ms - skipping startup planning",
                        total_waited
                    );
                } else {
                    eprintln!(
                        "[Planning] Found {} MCP tools after {}ms - starting autonomous planning",
                        tools.len(),
                        total_waited
                    );
                }

                if !tools.is_empty() && !planning_completed.load(Ordering::SeqCst) {
                    planning_completed.store(true, Ordering::SeqCst);

                    // Get agent's configured model for planning
                    let preferred_model = {
                        let metadata = agent_metadata.read().await;
                        arkavo_router::ModelChoice::from_name(&metadata.model)
                    };

                    let plan = run_startup_planning_phase(
                        &system_prompt,
                        &router,
                        &mcp_registry,
                        &conductor,
                        preferred_model,
                    )
                    .await;

                    // Add planning summary to shared context so agent remembers what it planned
                    {
                        let goals_summary = plan
                            .goals
                            .iter()
                            .enumerate()
                            .map(|(i, g)| format!("{}. {}", i + 1, g.description))
                            .collect::<Vec<_>>()
                            .join("\n");
                        let planning_message = format!(
                            "I have analyzed my purpose and created the following goals:\n{}\n\nI will watch for: {:?}",
                            goals_summary, plan.watch_for
                        );
                        agent_context.write().await.push(arkavo_llm::Message {
                            role: arkavo_llm::Role::Assistant,
                            content: planning_message,
                            images: None,
                        });
                    }

                    *agent_plan.write().await = plan;
                    eprintln!("[Planning] Agent startup planning complete");

                    // Execute first goal immediately if available
                    let plan_guard = agent_plan.read().await;
                    if let Some(first_goal) = plan_guard.goals.first() {
                        eprintln!(
                            "[Planning] Executing first goal: {}",
                            first_goal.description
                        );
                        let goal_prompt = format!(
                            "{}\n\n## Immediate Task\n{}\n\nExecute this goal now using the available tools.",
                            system_prompt, first_goal.description
                        );
                        drop(plan_guard);

                        // Execute the first goal using the conductor with agent's configured model
                        let preferred_model = {
                            let metadata = agent_metadata.read().await;
                            arkavo_router::ModelChoice::from_name(&metadata.model)
                        };
                        if preferred_model.is_some() {
                            eprintln!("[Planning] Using configured model: {:?}", preferred_model);
                        }
                        match execute_with_conductor_and_model(
                            &conductor,
                            &router,
                            &mcp_registry,
                            goal_prompt.clone(),
                            preferred_model,
                        )
                        .await
                        {
                            Ok(response) => {
                                eprintln!(
                                    "[Planning] First goal response: {}",
                                    response.chars().take(200).collect::<String>()
                                );
                                // Add goal execution to context
                                agent_context.write().await.push(arkavo_llm::Message {
                                    role: arkavo_llm::Role::Assistant,
                                    content: format!("Executed first goal. Result: {}", response),
                                    images: None,
                                });
                            }
                            Err(e) => {
                                eprintln!("[Planning] First goal execution failed: {}", e);
                                // Add failure to context
                                agent_context.write().await.push(arkavo_llm::Message {
                                    role: arkavo_llm::Role::Assistant,
                                    content: format!("First goal execution failed: {}", e),
                                    images: None,
                                });
                            }
                        }
                    }
                }
            }

            // Agent action loop interval - from agent configuration (default: 10 seconds)
            let action_interval_secs = {
                let metadata = agent_metadata.read().await;
                if metadata.action_interval > 0 {
                    metadata.action_interval
                } else {
                    10 // Default to 10 seconds for autonomous operation
                }
            };
            eprintln!("[Goals] Action loop interval: {}s", action_interval_secs);

            let mut action_tick = {
                let mut interval =
                    tokio::time::interval(tokio::time::Duration::from_secs(action_interval_secs));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                Some(interval)
            };

            loop {
                tokio::select! {
                    // Goal-driven action tick - work toward completing plan goals
                    _ = async {
                        if let Some(ref mut tick) = action_tick {
                            tick.tick().await
                        } else {
                            std::future::pending::<tokio::time::Instant>().await
                        }
                    } => {
                        // Find next pending goal to work on
                        let mut plan = agent_plan.write().await;
                        let current_goal = plan.goals.iter_mut()
                            .find(|g| matches!(g.status, super::startup::GoalStatus::Active | super::startup::GoalStatus::InProgress));

                        let goal_prompt = if let Some(goal) = current_goal {
                            goal.status = super::startup::GoalStatus::InProgress;
                            let goal_desc = goal.description.clone();
                            drop(plan);

                            let memory = agent_memory.read().await;
                            let memory_section = memory.format_for_prompt();
                            drop(memory);

                            eprintln!("[Goals] Working on: {}", goal_desc);
                            Some(format!(
                                "{}\n\n## Current Goal\n{}\n\n## Recent Actions (review before acting)\n{}\n\n## Instructions\nReview recent actions above - do NOT repeat actions that already succeeded or failed. Make progress on the goal using different tools. When the goal is achieved, respond with GOAL_COMPLETE.",
                                system_prompt,
                                goal_desc,
                                memory_section
                            ))
                        } else {
                            drop(plan);
                            // All goals complete - replan to generate new objectives
                            eprintln!("[Goals] All goals complete - re-planning for new objectives");

                            let preferred_model = {
                                let metadata = agent_metadata.read().await;
                                arkavo_router::ModelChoice::from_name(&metadata.model)
                            };

                            let new_plan = run_startup_planning_phase(
                                &system_prompt,
                                &router,
                                &mcp_registry,
                                &conductor,
                                preferred_model,
                            )
                            .await;

                            if !new_plan.goals.is_empty() {
                                eprintln!("[Goals] Re-planned {} new goals", new_plan.goals.len());
                                *agent_plan.write().await = new_plan;
                            }
                            None
                        };

                        if let Some(prompt) = goal_prompt {
                            // Use agent's configured model
                            let preferred_model = {
                                let metadata = agent_metadata.read().await;
                                arkavo_router::ModelChoice::from_name(&metadata.model)
                            };
                            match execute_with_conductor_and_model(
                                &conductor,
                                &router,
                                &mcp_registry,
                                prompt,
                                preferred_model,
                            )
                            .await
                            {
                                Ok(result) => {
                                    eprintln!("[Goals] Result: {} chars", result.len());

                                    // Check if goal was completed
                                    if result.contains("GOAL_COMPLETE") {
                                        let mut plan = agent_plan.write().await;
                                        if let Some(goal) = plan.goals.iter_mut()
                                            .find(|g| matches!(g.status, super::startup::GoalStatus::InProgress))
                                        {
                                            goal.status = super::startup::GoalStatus::Completed;
                                            eprintln!("[Goals] Completed: {}", goal.description);
                                        }
                                    }

                                    // Add to context
                                    if !result.is_empty() {
                                        agent_context.write().await.push(arkavo_llm::Message {
                                            role: arkavo_llm::Role::Assistant,
                                            content: result.chars().take(500).collect::<String>(),
                                            images: None,
                                        });
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[Goals] Action failed: {}", e);
                                }
                            }
                        }
                    }

                    // Handle MCP notifications (reactive)
                    notification_result = notification_rx.recv() => {
                        let notification = match notification_result {
                            Ok(n) => n,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                warn!("Notification handler lagged, missed {} messages", n);
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                info!("Notification channel closed, stopping handler");
                                break;
                            }
                        };

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

                        // Planning already done at startup, skip redundant check

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
                        // Use agent's configured model
                        let preferred_model = {
                            let metadata = agent_metadata.read().await;
                            arkavo_router::ModelChoice::from_name(&metadata.model)
                        };
                        let start_time = std::time::Instant::now();
                        match execute_with_conductor_and_model(
                            &conductor,
                            &router,
                            &mcp_registry,
                            prompt,
                            preferred_model,
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

                                // Check for REQUEST_HELP pattern for A2A collaboration
                                if let Some(help_start) = result.find("REQUEST_HELP:") {
                                    let help_desc = result[help_start + 13..]
                                        .lines()
                                        .next()
                                        .unwrap_or("")
                                        .trim();
                                    if !help_desc.is_empty() {
                                        info!(
                                            "[A2A] Agent requesting help: {}",
                                            help_desc
                                        );
                                        // TODO: Route to appropriate peer via A2A
                                        // For now, log the request - full implementation
                                        // would query peers and inject response
                                    }
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
                } // tokio::select!
            }
        });

        Some(handle)
    }

    pub async fn start(&self) -> Result<ServerHandle> {
        let addr: SocketAddr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| A2aError::InvalidEndpoint(format!("Invalid bind address: {e}")))?;

        info!("Starting A2A server on {}", addr);

        // Rebuild tool registry now that MCP servers have been registered
        // This fixes the race condition where tools were queried before MCP servers connected
        self.rebuild_tool_registry().await;

        let server = ServerBuilder::default()
            .max_connections(self.config.max_connections as u32)
            .build(addr)
            .await
            .map_err(|e| A2aError::Transport(format!("Failed to build server: {e}")))?;

        let rate_limiter = Arc::new(RateLimiter::new(self.config.rate_limit.clone()));
        let metrics = Arc::new(MetricsCollector::new(self.config.metrics_enabled));
        let llm_adapter = self.llm_adapter.read().await.clone();
        let router = self.router.read().await.clone();
        let tool_registry = self.tool_registry.read().await.clone();

        // Initialize agent context with system prompt if not already set
        {
            let mut ctx = self.agent_context.write().await;
            if ctx.is_empty() {
                let metadata = self.agent_metadata.read().await;
                if !metadata.purpose.is_empty() {
                    ctx.push(arkavo_llm::Message {
                        role: arkavo_llm::Role::System,
                        content: metadata.purpose.clone(),
                        images: None,
                    });
                    info!("Initialized agent context with purpose as system prompt");
                }
            }
        }

        // Use shared agent context for all chat sessions
        let shared_context = self.agent_context.clone();
        let tool_memory = Some(self.agent_memory.clone());
        let agent_plan = Some(self.agent_plan.clone());

        // Get preferred model from agent metadata for chat sessions
        let preferred_model = {
            let metadata = self.agent_metadata.read().await;
            arkavo_router::ModelChoice::from_name(&metadata.model)
        };
        if preferred_model.is_some() {
            info!(
                "Chat sessions will use preferred model: {:?}",
                preferred_model
            );
        }

        let chat_sessions = if let Some(router_instance) = router.clone() {
            info!(
                "✓ ChatSessionManager will be created WITH Router + shared context + memory + plan"
            );
            Arc::new(crate::chat_session::ChatSessionManager::with_full_context(
                None,
                Some(router_instance),
                tool_registry.clone(),
                3600,
                self.buffer_config.clone(),
                shared_context,
                tool_memory,
                agent_plan,
                preferred_model,
            ))
        } else if llm_adapter.is_some() {
            info!("✓ ChatSessionManager will be created WITH LLM adapter + shared context");
            Arc::new(crate::chat_session::ChatSessionManager::with_full_context(
                llm_adapter.clone(),
                None,
                None,
                3600,
                self.buffer_config.clone(),
                shared_context,
                tool_memory,
                agent_plan,
                preferred_model,
            ))
        } else {
            warn!(
                "✗ ChatSessionManager will be created WITHOUT LLM adapter or router - messages will fail!"
            );
            Arc::new(crate::chat_session::ChatSessionManager::with_full_context(
                None,
                None,
                None,
                3600,
                self.buffer_config.clone(),
                shared_context,
                tool_memory,
                agent_plan,
                preferred_model,
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
        };

        if let Err(e) = self.start_file_watcher().await {
            warn!("Failed to start file watcher: {}", e);
        }

        let handle = server.start(rpc_impl.into_rpc());

        info!("A2A server started successfully on {}", addr);
        info!("OpenRPC schema available via JSON-RPC method: rpc.discover");

        Ok(handle)
    }
}
