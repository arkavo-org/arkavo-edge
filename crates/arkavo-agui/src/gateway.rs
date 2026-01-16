use crate::agent_connection::{AgentConnection, TelemetryEvent};
use crate::budget_handler::BudgetHandler;
use crate::dataflow_handler::DataflowHandler;
use crate::debug_handler::DebugHandler;
use crate::types::*;
use arkavo_observability::metrics_snapshot::{MetricsSampler, MetricsSamplerConfig};
use arkavo_protocol::types::ConfigError;
use axum::{
    Json, Router,
    extract::{
        Path as AxumPath, State,
        ws::{Message, WebSocket},
    },
    response::{
        Html, IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

/// Connection information for active WebSocket clients
struct ConnectionInfo {
    _ws_tx: mpsc::Sender<AgUiEvent>,
    _agent_id: Option<String>,
    subscriptions: Vec<SubscriptionHandle>,
    current_plan: Option<Vec<arkavo_ui_generator::planner::ComponentPart>>,
    current_prompt: Option<String>,
}

#[derive(Clone)]
struct AppState {
    connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    budget_handler: Arc<RwLock<BudgetHandler>>,
    initial_prompt: Arc<RwLock<Option<String>>>,
    dataflow_handler: Arc<DataflowHandler>,
    telemetry_rx: Arc<RwLock<mpsc::Receiver<TelemetryEvent>>>,
    debug_handler: Option<Arc<DebugHandler>>,
}

pub struct AgUiGateway {
    port: u16,
    discovered_agents: Arc<RwLock<Vec<serde_json::Value>>>,
    connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
    telemetry_rx: Option<mpsc::Receiver<TelemetryEvent>>,
    dataflow_handler: Arc<DataflowHandler>,
    budget_handler: Arc<RwLock<BudgetHandler>>,
    debug_handler: Option<Arc<DebugHandler>>,
    initial_prompt: Option<String>,
}

impl AgUiGateway {
    pub fn new(port: u16) -> Self {
        let (telemetry_tx, telemetry_rx) = mpsc::channel(1000);
        Self {
            port,
            discovered_agents: Arc::new(RwLock::new(Vec::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            agent_connections: Arc::new(RwLock::new(HashMap::new())),
            telemetry_tx,
            telemetry_rx: Some(telemetry_rx),
            dataflow_handler: Arc::new(DataflowHandler::new()),
            budget_handler: Arc::new(RwLock::new(BudgetHandler::new())),
            debug_handler: None,
            initial_prompt: None,
        }
    }

    pub fn set_initial_prompt(&mut self, prompt: String) {
        self.initial_prompt = Some(prompt);
    }

    pub async fn with_debug_handler(
        mut self,
        storage: arkavo_memory::storage::MemoryStorage,
    ) -> Self {
        self.debug_handler = Some(Arc::new(DebugHandler::new(Arc::new(storage)).await));
        self
    }

    pub async fn start(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let discovered_agents = self.discovered_agents.clone();
        let agents_clone = discovered_agents.clone();

        // Initialize budget handler with default configuration
        {
            let budget_config = arkavo_budget::BudgetConfig::default();
            let (budget_tx, mut budget_rx) = mpsc::channel::<AgUiEvent>(100);

            let mut handler = self.budget_handler.write().await;
            handler.initialize(budget_config, budget_tx).await?;

            // Forward budget events to all connected clients
            let connections = self.connections.clone();
            tokio::spawn(async move {
                while let Some(event) = budget_rx.recv().await {
                    let conns = connections.read().await;
                    for (_, conn_info) in conns.iter() {
                        let _ = conn_info._ws_tx.send(event.clone()).await;
                    }
                }
            });
        }

        // Start mDNS discovery in background
        let agent_connections_for_mdns = self.agent_connections.clone();
        let telemetry_tx_for_mdns = self.telemetry_tx.clone();
        tokio::spawn(async move {
            println!("AG-UI: Starting mDNS discovery...");
            match crate::gateway_mdns::run_mdns_discovery(
                agents_clone,
                agent_connections_for_mdns,
                telemetry_tx_for_mdns,
            )
            .await
            {
                Ok(_) => println!("AG-UI: mDNS discovery completed"),
                Err(e) => eprintln!("AG-UI: mDNS discovery error: {e}"),
            }
        });

        // Start metrics sampler task
        let telemetry_tx_for_metrics = self.telemetry_tx.clone();
        tokio::spawn(async move {
            println!("AG-UI: Starting metrics sampler...");
            let config = MetricsSamplerConfig::default();
            let mut sampler = MetricsSampler::new_with_name(config, "arkavo-agui".to_string());

            // Create a mock metrics collector for now
            // In production, this would be integrated with actual metrics
            let metrics_collector = arkavo_observability::metrics::MetricsCollector::new();

            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                interval.tick().await;

                let snapshot = sampler.sample_metrics(&metrics_collector);

                // Send metrics through telemetry channel
                let _ = telemetry_tx_for_metrics
                    .send(TelemetryEvent::MetricsSnapshot { snapshot })
                    .await;
            }
        });

        // Register health reporters with global registry
        {
            use arkavo_observability::health_reporter::HealthRegistry;
            use std::sync::Arc;

            let registry = HealthRegistry::global();

            // Register AG-UI health reporter
            let agui_reporter = Arc::new(crate::health::AguiHealthReporter::new());
            registry.register(agui_reporter).await;

            // Register Router health reporter
            use arkavo_router::health::RouterHealthReporter;
            let connectivity_checker = Arc::new(arkavo_router::ConnectivityChecker::new());
            let router_reporter = Arc::new(RouterHealthReporter::new(connectivity_checker));
            registry.register(router_reporter).await;

            // Register UI Generator health reporter (wrapper that delegates to global instance)
            use arkavo_ui_generator::health::GlobalHealthReporterWrapper;
            let ui_generator_reporter = Arc::new(GlobalHealthReporterWrapper);
            registry.register(ui_generator_reporter).await;

            println!("AG-UI: Health reporters registered");
        }

        // Create channel for broadcasting health alerts to all WebSocket connections
        let (health_alert_tx, mut health_alert_rx) = mpsc::channel::<AgUiEvent>(100);

        // Start intelligent health monitor with local LLM
        {
            use arkavo_mcp_tools::registry::ToolRegistry;
            use arkavo_memory::MemoryStorage;

            let storage = Arc::new(MemoryStorage::new().await?);
            let tool_registry = Arc::new(ToolRegistry::new(storage));

            let health_monitor = crate::health_monitor::HealthMonitor::new(tool_registry)
                .await?
                .with_interval(30); // Check every 30 seconds

            let _health_task = health_monitor.start(health_alert_tx.clone()).await?;
            println!("AG-UI: Health monitor started (30s interval, model selection via router)");
        }

        // Start command health collector with LLM-based timeout analysis
        {
            use crate::command_health_collector::CommandHealthCollector;
            use crate::timeout_handler::TimeoutHandler;

            let (command_health_tx, command_health_rx) = mpsc::channel(100);

            // Start collector that batches health data every 30 seconds
            let collector = CommandHealthCollector::new(30);
            let _collector_task = collector.start(command_health_tx).await?;

            // Start timeout analyzer that uses local LLM to decide timeouts
            let timeout_handler = TimeoutHandler::new().await?;
            let _timeout_task = timeout_handler
                .start(command_health_rx, health_alert_tx.clone())
                .await;

            println!(
                "AG-UI: Command health collector started (30s batching, LLM-based timeout analysis)"
            );
        }

        // Broadcast health alerts to all connected WebSocket clients
        let connections_for_health = self.connections.clone();
        tokio::spawn(async move {
            while let Some(alert_event) = health_alert_rx.recv().await {
                let conns = connections_for_health.read().await;
                for (_, conn_info) in conns.iter() {
                    let _ = conn_info._ws_tx.send(alert_event.clone()).await;
                }
            }
        });

        // Push periodic status updates every 30 seconds
        let connections_for_status = self.connections.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            loop {
                interval.tick().await;

                use arkavo_mcp_tools::registry::ToolRegistry;
                use arkavo_memory::MemoryStorage;
                use arkavo_observability::health_reporter::HealthRegistry;
                use arkavo_router::Router;

                let storage = match MemoryStorage::new().await {
                    Ok(s) => Arc::new(s),
                    Err(e) => {
                        eprintln!("Failed to initialize storage for health status: {e}");
                        continue;
                    }
                };
                let registry = ToolRegistry::new(storage);
                let tools = registry.list_tools();
                let browser_tool = tools.iter().find(|t| t.name.contains("browser"));

                // Get health data
                let health_registry = HealthRegistry::global();
                let health_reports = health_registry.check_all().await;
                let overall_health = health_registry.get_overall_status().await;

                let conns = connections_for_status.read().await;

                let system_status = SystemStatus {
                    uptime: format!(
                        "{} seconds",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    ),
                    memory_usage: "N/A".to_string(),
                    active_connections: conns.len() as u32,
                };

                let mcp_tools_status = McpToolsStatus {
                    browser_available: browser_tool.is_some(),
                    tools_count: tools.len(),
                    last_used: None,
                };

                if let Ok(router) = Router::new().await {
                    let llms = router
                        .get_available_llms()
                        .into_iter()
                        .map(|info| LlmStatus {
                            name: info.name,
                            provider: info.provider,
                            connected: info.available,
                            model: info.model,
                            requests_today: 0,
                        })
                        .collect();

                    let health_data = HealthData {
                        status: format!("{:?}", overall_health),
                        components: health_reports
                            .iter()
                            .map(|r| ComponentHealth {
                                component: r.component.clone(),
                                status: format!("{:?}", r.status),
                                message: r.message.clone(),
                            })
                            .collect(),
                    };

                    let status_event = AgUiEvent::StatusUpdate {
                        system: system_status,
                        mcp_tools: mcp_tools_status,
                        llms,
                        health: health_data,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    };

                    // Broadcast to all connections
                    for (_, conn_info) in conns.iter() {
                        let _ = conn_info._ws_tx.send(status_event.clone()).await;
                    }

                    println!(
                        "AG-UI: Pushed status update - Health: {:?}, Components: {}",
                        overall_health,
                        health_reports.len()
                    );
                }
            }
        });

        // Monitor agent changes and broadcast AgentDiscovered/AgentLost events
        let connections_for_agents = self.connections.clone();
        let agents_for_monitor = discovered_agents.clone();
        tokio::spawn(async move {
            let mut previous_agents: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));

            loop {
                interval.tick().await;

                let agents_list = agents_for_monitor.read().await;
                let current_agents: std::collections::HashSet<String> = agents_list
                    .iter()
                    .filter_map(|a| a.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .collect();

                // Find new agents
                for agent_id in current_agents.difference(&previous_agents) {
                    if let Some(agent) = agents_list
                        .iter()
                        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(agent_id.as_str()))
                    {
                        let event = AgUiEvent::AgentDiscovered {
                            agent_id: agent_id.clone(),
                            endpoint: agent
                                .get("endpoint")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            purpose: agent
                                .get("purpose")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            model: agent
                                .get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        };

                        let conns = connections_for_agents.read().await;
                        for (_, conn_info) in conns.iter() {
                            let _ = conn_info._ws_tx.send(event.clone()).await;
                        }
                        println!("AG-UI: Broadcast AgentDiscovered: {}", agent_id);
                    }
                }

                // Find lost agents
                for agent_id in previous_agents.difference(&current_agents) {
                    let event = AgUiEvent::AgentLost {
                        agent_id: agent_id.clone(),
                        reason: "Agent no longer discovered".to_string(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    };

                    let conns = connections_for_agents.read().await;
                    for (_, conn_info) in conns.iter() {
                        let _ = conn_info._ws_tx.send(event.clone()).await;
                    }
                    println!("AG-UI: Broadcast AgentLost: {}", agent_id);
                }

                previous_agents = current_agents;
            }
        });

        // Create shared state for handlers
        let telemetry_rx = self
            .telemetry_rx
            .take()
            .expect("telemetry_rx already taken");

        let state = AppState {
            connections: self.connections.clone(),
            agents: discovered_agents.clone(),
            agent_connections: self.agent_connections.clone(),
            budget_handler: self.budget_handler.clone(),
            initial_prompt: Arc::new(RwLock::new(self.initial_prompt.clone())),
            dataflow_handler: self.dataflow_handler.clone(),
            telemetry_rx: Arc::new(RwLock::new(telemetry_rx)),
            debug_handler: self.debug_handler.clone(),
        };

        // Build axum router
        let app = Router::new()
            .route("/", get(index_handler))
            .route("/static/toolbar.js", get(static_js_handler))
            .route("/chat", get(chat_ui_handler))
            .route("/ws", get(websocket_handler))
            .route("/events", get(sse_handler))
            .route("/agent/:id", post(agent_proxy_handler))
            .route("/api/dataflow/*path", post(dataflow_handler))
            .route("/telemetry", get(telemetry_websocket_handler))
            .route("/debug", get(debug_websocket_handler))
            .with_state(state);

        let addr: SocketAddr = ([0, 0, 0, 0], self.port).into();
        println!("Starting AG-UI Gateway on http://127.0.0.1:{}", self.port);
        println!("WebSocket endpoint: ws://127.0.0.1:{}/ws", self.port);
        println!(
            "Dataflow API endpoint: http://127.0.0.1:{}/api/dataflow",
            self.port
        );
        println!("Open http://127.0.0.1:{} in your web browser", self.port);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

async fn index_handler() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn static_js_handler() -> Response {
    use axum::{body::Body, http::header};
    Response::builder()
        .header(header::CONTENT_TYPE, "application/javascript")
        .body(Body::from(include_str!("../static/toolbar.js")))
        .unwrap()
        .into_response()
}

async fn chat_ui_handler() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| {
        handle_websocket(
            socket,
            state.connections,
            state.agents,
            state.agent_connections,
            state.budget_handler,
            state.initial_prompt,
        )
    })
}

async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    use futures::stream;

    let agents = state.agents;
    let event_stream = stream::unfold(agents, |agents| async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let agents_list = agents.read().await.clone();
        let response = serde_json::json!({ "agents": agents_list });

        let event = Event::default()
            .event("discovery")
            .data(response.to_string());

        Some((Ok::<_, Infallible>(event), agents))
    });

    Sse::new(event_stream)
}

async fn agent_proxy_handler(
    AxumPath(agent_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    handle_agent_proxy(agent_id, body, state.agents).await
}

async fn dataflow_handler(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let path_vec = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect::<Vec<String>>();

    state.dataflow_handler.handle_request(path_vec, body).await
}

async fn telemetry_websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| handle_telemetry_websocket(socket, state.telemetry_rx))
}

async fn debug_websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| handle_debug_websocket(socket, state.debug_handler))
}

async fn handle_websocket(
    ws: WebSocket,
    connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    budget_handler: Arc<RwLock<BudgetHandler>>,
    initial_prompt: Arc<RwLock<Option<String>>>,
) {
    use futures::sink::SinkExt;
    use futures::stream::StreamExt;

    let session_id = uuid::Uuid::new_v4().to_string();

    // Create bounded channel for back-pressure handling
    let (tx, mut rx) = mpsc::channel::<AgUiEvent>(32);

    // Split WebSocket into read and write halves to avoid deadlock
    let (mut ws_write, mut ws_read) = ws.split();

    // Spawn task to forward messages from channel to WebSocket write half
    let forward_task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                if ws_write.send(Message::Text(json)).await.is_err() {
                    break; // WebSocket closed
                }
            }
        }
    });

    println!("AG-UI: New WebSocket connection: {session_id}");

    // Register connection immediately so it receives broadcasts (heartbeats, agent discovery)
    {
        let conn_info = ConnectionInfo {
            _ws_tx: tx.clone(),
            _agent_id: None,
            subscriptions: Vec::new(),
            current_plan: None,
            current_prompt: None,
        };
        connections
            .write()
            .await
            .insert(session_id.clone(), conn_info);
    }

    // If there's an initial prompt, send it automatically
    let prompt_guard = initial_prompt.read().await;
    if let Some(prompt_text) = prompt_guard.as_ref() {
        println!("AG-UI: Auto-submitting initial prompt: {prompt_text}");
        let submit_event = AgUiEvent::SubmitPrompt {
            text: prompt_text.clone(),
        };
        drop(prompt_guard);

        if let Err(e) = handle_event(
            submit_event,
            &session_id,
            &connections,
            &agents,
            &agent_connections,
            &budget_handler,
            &tx,
        )
        .await
        {
            eprintln!("AG-UI: Error auto-submitting initial prompt: {e}");
        }
    }

    // Handle incoming messages from WebSocket read half
    loop {
        let msg = ws_read.next().await;

        match msg {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<AgUiEvent>(&text) {
                Ok(event) => {
                    if let Err(e) = handle_event(
                        event,
                        &session_id,
                        &connections,
                        &agents,
                        &agent_connections,
                        &budget_handler,
                        &tx,
                    )
                    .await
                    {
                        eprintln!("AG-UI: Error handling event: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("AG-UI: Failed to parse event: {e}");
                    let error = AgUiEvent::Error {
                        code: "INVALID_EVENT".to_string(),
                        message: format!("Failed to parse event: {e}"),
                    };
                    let _ = tx.send(error).await;
                }
            },
            Some(Ok(Message::Close(_))) | None => {
                break;
            }
            Some(Err(e)) => {
                eprintln!("AG-UI: WebSocket error: {e}");
                break;
            }
            _ => {}
        }
    }

    // Clean up connection and subscriptions
    if let Some(mut conn_info) = connections.write().await.remove(&session_id) {
        // Cancel all subscriptions
        for mut sub in conn_info.subscriptions.drain(..) {
            sub.cancel();
        }
    }

    // Cancel the forward task
    forward_task.abort();

    println!("AG-UI: WebSocket connection closed: {session_id}");
}

async fn handle_event(
    event: AgUiEvent,
    session_id: &str,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    budget_handler: &Arc<RwLock<BudgetHandler>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    match event {
        AgUiEvent::Connect {
            agent_id,
            agui_version,
            since_event_id: _,
        } => {
            println!("AG-UI: Connect request for agent {agent_id} (version {agui_version})");

            // Find the agent
            let agents_list = agents.read().await;
            let agent = agents_list
                .iter()
                .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(&agent_id))
                .cloned();

            if let Some(_agent_info) = agent {
                // Store connection info
                let conn_info = ConnectionInfo {
                    _ws_tx: tx.clone(),
                    _agent_id: Some(agent_id.clone()),
                    subscriptions: Vec::new(),
                    current_plan: None,
                    current_prompt: None,
                };

                connections
                    .write()
                    .await
                    .insert(session_id.to_string(), conn_info);

                println!("AG-UI: Stored connection info for session {session_id}");

                // Send initial snapshots
                let state_snapshot = AgUiEvent::StateSnapshot {
                    state: serde_json::json!({}),
                    event_id: uuid::Uuid::new_v4().to_string(),
                };

                let messages_snapshot = AgUiEvent::MessagesSnapshot {
                    messages: vec![],
                    event_id: uuid::Uuid::new_v4().to_string(),
                };

                tx.send(state_snapshot).await?;
                tx.send(messages_snapshot).await?;

                // Send lifecycle start
                let lifecycle_start = AgUiEvent::LifecycleStart {
                    session_id: session_id.to_string(),
                };

                tx.send(lifecycle_start).await?;
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_FOUND".to_string(),
                    message: format!("Agent {agent_id} not found"),
                };
                tx.send(error).await?;
            }
        }

        AgUiEvent::ChatOpen { agent_id } => {
            // Get the agent connection
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                // Subscribe to chat for this agent
                match agent_conn
                    .subscribe_to_chat(agent_id.clone(), tx.clone(), None) // See #384
                    .await
                {
                    Ok(sub_handle) => {
                        // Store subscription handle
                        let mut conn_guard = connections.write().await;
                        if let Some(conn_info) = conn_guard.get_mut(session_id) {
                            conn_info.subscriptions.push(sub_handle);
                        }
                    }
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "SUBSCRIPTION_FAILED".to_string(),
                            message: format!("Failed to subscribe to chat: {e}"),
                        };
                        tx.send(error).await?;
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        AgUiEvent::ChatClose { agent_id } => {
            // Get the agent connection
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                // Unsubscribe from chat
                if let Err(e) = agent_conn.unsubscribe_chat(&agent_id).await {
                    eprintln!("Failed to unsubscribe from chat: {e}");
                }
            }
        }

        AgUiEvent::UserMessage {
            agent_id,
            content,
            attachments: _,
        } => {
            // Get the agent connection
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                // Send the user message
                match agent_conn.send_user_message(&agent_id, content).await {
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "MESSAGE_SEND_FAILED".to_string(),
                            message: format!("Failed to send message: {e}"),
                        };
                        tx.send(error).await?;
                    }
                    Ok(_) => {
                        // Message sent successfully
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        // Budget-related events
        AgUiEvent::GetBudgetStatus { .. }
        | AgUiEvent::SetAgentBudget { .. }
        | AgUiEvent::ResetBudgetWindow { .. } => {
            let handler = budget_handler.read().await;
            handler.handle_event(&event, tx).await?;
        }

        // Configuration management events
        AgUiEvent::GetAgentConfig {
            agent_id,
            include_backups,
        } => {
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                match agent_conn.get_config(include_backups).await {
                    Ok(response) => {
                        let event = AgUiEvent::AgentConfigSnapshot {
                            content: response.content,
                            version: response.version,
                            backups: response.backups.map(|backups| {
                                backups
                                    .into_iter()
                                    .map(|b| ConfigBackupInfo {
                                        filename: b.filename,
                                        timestamp: b.timestamp,
                                        size: b.size,
                                        version: b.version,
                                    })
                                    .collect()
                            }),
                            writable: response.writable,
                        };
                        tx.send(event).await?;
                    }
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "CONFIG_GET_FAILED".to_string(),
                            message: format!("Failed to get configuration: {e}"),
                        };
                        tx.send(error).await?;
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        AgUiEvent::UpdateAgentConfig {
            agent_id,
            content,
            expected_version,
            create_backup,
        } => {
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                match agent_conn
                    .update_config(content, expected_version, create_backup)
                    .await
                {
                    Ok(response) => {
                        let event = AgUiEvent::ConfigUpdateResult {
                            success: response.success,
                            new_version: response.new_version,
                            backup_path: response.backup_path,
                            error: response.error.map(|e| match e {
                                ConfigError::AgentOffline => ConfigErrorInfo::AgentOffline,
                                ConfigError::ReadOnlyFilesystem => {
                                    ConfigErrorInfo::ReadOnlyFilesystem
                                }
                                ConfigError::ValidationFailed { details } => {
                                    ConfigErrorInfo::ValidationFailed { details }
                                }
                                ConfigError::Conflict { current_version } => {
                                    ConfigErrorInfo::Conflict { current_version }
                                }
                                ConfigError::Unauthorized => ConfigErrorInfo::Unauthorized,
                            }),
                            reload_required: response.reload_required,
                        };
                        tx.send(event).await?;
                    }
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "CONFIG_UPDATE_FAILED".to_string(),
                            message: format!("Failed to update configuration: {e}"),
                        };
                        tx.send(error).await?;
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        AgUiEvent::ValidateAgentConfig { agent_id, content } => {
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                match agent_conn.validate_config(content).await {
                    Ok(response) => {
                        let event = AgUiEvent::ConfigValidationResult {
                            valid: response.valid,
                            errors: response.errors,
                            warnings: response.warnings,
                        };
                        tx.send(event).await?;
                    }
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "CONFIG_VALIDATE_FAILED".to_string(),
                            message: format!("Failed to validate configuration: {e}"),
                        };
                        tx.send(error).await?;
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        AgUiEvent::RestoreAgentConfig {
            agent_id,
            backup_filename,
        } => {
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                match agent_conn.restore_config(backup_filename).await {
                    Ok(response) => {
                        let event = AgUiEvent::ConfigRestoreResult {
                            success: response.success,
                            new_version: response.new_version,
                            error: response.error.map(|e| match e {
                                ConfigError::AgentOffline => ConfigErrorInfo::AgentOffline,
                                ConfigError::ReadOnlyFilesystem => {
                                    ConfigErrorInfo::ReadOnlyFilesystem
                                }
                                ConfigError::ValidationFailed { details } => {
                                    ConfigErrorInfo::ValidationFailed { details }
                                }
                                ConfigError::Conflict { current_version } => {
                                    ConfigErrorInfo::Conflict { current_version }
                                }
                                ConfigError::Unauthorized => ConfigErrorInfo::Unauthorized,
                            }),
                        };
                        tx.send(event).await?;
                    }
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "CONFIG_RESTORE_FAILED".to_string(),
                            message: format!("Failed to restore configuration: {e}"),
                        };
                        tx.send(error).await?;
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        // UI Generation events
        AgUiEvent::SubmitPrompt { text } => {
            println!("AG-UI: Received SubmitPrompt: {text}");

            // Extract and set any API keys from the prompt
            let (cleaned_text, api_keys) = crate::api_keys::extract_api_keys(&text);
            if !api_keys.is_empty() {
                crate::api_keys::set_api_keys(&api_keys);

                // Send updated status immediately to reflect new LLM availability
                use arkavo_mcp_tools::registry::ToolRegistry;
                use arkavo_memory::MemoryStorage;
                use arkavo_router::Router;

                let storage = match MemoryStorage::new().await {
                    Ok(s) => Arc::new(s),
                    Err(e) => {
                        eprintln!("Failed to initialize storage for status update: {e}");
                        return Ok(());
                    }
                };
                let registry = ToolRegistry::new(storage);
                let tools = registry.list_tools();
                let browser_tool = tools.iter().find(|t| t.name.contains("browser"));

                let system_status = SystemStatus {
                    uptime: format!(
                        "{} seconds",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    ),
                    memory_usage: "N/A".to_string(),
                    active_connections: connections.read().await.len() as u32,
                };

                let mcp_tools_status = McpToolsStatus {
                    browser_available: browser_tool.is_some(),
                    tools_count: tools.len(),
                    last_used: None,
                };

                // Get health data
                use arkavo_observability::health_reporter::HealthRegistry;
                let health_registry = HealthRegistry::global();
                let health_reports = health_registry.check_all().await;
                let overall_health = health_registry.get_overall_status().await;

                let router = Router::new().await?;
                let llms = router
                    .get_available_llms()
                    .into_iter()
                    .map(|info| LlmStatus {
                        name: info.name,
                        provider: info.provider,
                        connected: info.available,
                        model: info.model,
                        requests_today: 0,
                    })
                    .collect();

                let health_data = HealthData {
                    status: format!("{:?}", overall_health),
                    components: health_reports
                        .iter()
                        .map(|r| ComponentHealth {
                            component: r.component.clone(),
                            status: format!("{:?}", r.status),
                            message: r.message.clone(),
                        })
                        .collect(),
                };

                let status_event = AgUiEvent::StatusUpdate {
                    system: system_status,
                    mcp_tools: mcp_tools_status,
                    llms,
                    health: health_data,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };

                tx.send(status_event).await?;
            }

            use arkavo_router::Router;
            use arkavo_ui_generator::planner::UiPlanner;

            // Note: UiPlanner uses Router for model selection but quality gate
            // integration is pending (requires UiPlanner refactoring)
            let router = Arc::new(Router::new().await?);
            let planner = UiPlanner::new(router);
            let plan = planner.plan(&cleaned_text).await?;

            let mut conn_guard = connections.write().await;
            if let Some(conn_info) = conn_guard.get_mut(session_id) {
                conn_info.current_plan = Some(plan.parts.clone());
                conn_info.current_prompt = Some(cleaned_text.clone());
            }
            drop(conn_guard);

            let parts: Vec<crate::types::UiPlanPart> = plan
                .parts
                .iter()
                .map(|p| crate::types::UiPlanPart {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    description: p.description.clone(),
                })
                .collect();

            println!(
                "AG-UI: Sending Plan event with {} parts to frontend",
                parts.len()
            );
            let plan_event = AgUiEvent::Plan { parts };
            tx.send(plan_event).await?;
            println!("AG-UI: Plan event sent successfully");
        }

        AgUiEvent::RequestStatus => {
            println!("AG-UI: Received RequestStatus");

            use arkavo_mcp_tools::registry::ToolRegistry;
            use arkavo_memory::MemoryStorage;
            use arkavo_observability::health_reporter::HealthRegistry;
            use arkavo_router::Router;

            let storage = match MemoryStorage::new().await {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    eprintln!("Failed to initialize storage for status request: {e}");
                    return Ok(());
                }
            };
            let registry = ToolRegistry::new(storage);
            let tools = registry.list_tools();
            let browser_tool = tools.iter().find(|t| t.name.contains("browser"));

            // Get health data from all registered components
            let health_registry = HealthRegistry::global();
            let health_reports = health_registry.check_all().await;
            let overall_health = health_registry.get_overall_status().await;

            let system_status = SystemStatus {
                uptime: format!(
                    "{} seconds",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                ),
                memory_usage: "N/A".to_string(),
                active_connections: connections.read().await.len() as u32,
            };

            let mcp_tools_status = McpToolsStatus {
                browser_available: browser_tool.is_some(),
                tools_count: tools.len(),
                last_used: None,
            };

            // Get all available LLMs from Router (single source of truth)
            let router = Router::new().await?;
            let llms = router
                .get_available_llms()
                .into_iter()
                .map(|info| LlmStatus {
                    name: info.name,
                    provider: info.provider,
                    connected: info.available,
                    model: info.model,
                    requests_today: 0,
                })
                .collect();

            let health_data = HealthData {
                status: format!("{:?}", overall_health),
                components: health_reports
                    .iter()
                    .map(|r| ComponentHealth {
                        component: r.component.clone(),
                        status: format!("{:?}", r.status),
                        message: r.message.clone(),
                    })
                    .collect(),
            };

            let status_event = AgUiEvent::StatusUpdate {
                system: system_status,
                mcp_tools: mcp_tools_status,
                llms,
                health: health_data,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };

            tx.send(status_event).await?;

            // Log health status for debugging
            println!(
                "AG-UI: Health Status - Overall: {:?}, Components: {}",
                overall_health,
                health_reports.len()
            );
            for report in &health_reports {
                println!(
                    "AG-UI:   - {} ({:?}): {}",
                    report.component, report.status, report.message
                );
            }
        }

        AgUiEvent::RequestMeshStatus => {
            println!("AG-UI: Received RequestMeshStatus");

            let agents_list = agents.read().await;
            let mesh_agents: Vec<crate::types::MeshAgentInfo> = agents_list
                .iter()
                .map(|a| crate::types::MeshAgentInfo {
                    id: a
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    endpoint: a
                        .get("endpoint")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    purpose: a
                        .get("purpose")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    model: a
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    status: "connected".to_string(),
                })
                .collect();

            let mesh_status = AgUiEvent::MeshStatus {
                agents: mesh_agents,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };

            // Debug: print JSON
            if let Ok(json) = serde_json::to_string(&mesh_status) {
                println!("AG-UI: MeshStatus JSON: {}", &json[..json.len().min(500)]);
            }
            tx.send(mesh_status).await?;
            println!("AG-UI: Sent MeshStatus with {} agents", agents_list.len());
        }

        AgUiEvent::ApplyPart { part_id } => {
            println!("AG-UI: Received ApplyPart: {part_id}");

            let conn_guard = connections.read().await;
            let conn_info = conn_guard.get(session_id);

            let (part_name, part_description, overall_prompt) = if let Some(info) = conn_info {
                let plan = info.current_plan.as_ref();
                let prompt = info.current_prompt.as_ref();

                let part = plan.and_then(|p| p.iter().find(|part| part.id == part_id));

                (
                    part.map(|p| p.name.clone())
                        .unwrap_or_else(|| "Component".to_string()),
                    part.map(|p| p.description.clone())
                        .unwrap_or_else(|| "UI Component".to_string()),
                    prompt
                        .cloned()
                        .unwrap_or_else(|| "Build a modern web component".to_string()),
                )
            } else {
                (
                    "Component".to_string(),
                    "UI Component".to_string(),
                    "Build a modern web component".to_string(),
                )
            };
            drop(conn_guard);

            use arkavo_router::Router;
            use arkavo_ui_generator::streaming::StreamingGenerator;

            // Note: StreamingGenerator uses Router for model selection but quality gate
            // integration is pending (requires StreamingGenerator refactoring)
            let router = Arc::new(Router::new().await?);
            let generator = StreamingGenerator::new(router)?;

            let mut stream_rx = generator
                .generate_part(&part_name, &part_description, &overall_prompt)
                .await?;

            let tx_clone = tx.clone();
            let part_id_clone = part_id.clone();
            tokio::spawn(async move {
                println!(
                    "AG-UI: Starting stream receiver task for part: {}",
                    part_id_clone
                );
                let mut chunk_count = 0;
                while let Some(chunk) = stream_rx.recv().await {
                    chunk_count += 1;
                    println!(
                        "AG-UI: Received chunk #{} for part {}, type={}, done={}, content_len={}",
                        chunk_count,
                        part_id_clone,
                        match chunk.chunk_type {
                            arkavo_ui_generator::streaming::ChunkType::Html => "html",
                            arkavo_ui_generator::streaming::ChunkType::Css => "css",
                            arkavo_ui_generator::streaming::ChunkType::JavaScript => "js",
                        },
                        chunk.done,
                        chunk.content.len()
                    );

                    let stream_event = AgUiEvent::PartStream {
                        part_id: part_id_clone.clone(),
                        chunk_type: match chunk.chunk_type {
                            arkavo_ui_generator::streaming::ChunkType::Html => "html".to_string(),
                            arkavo_ui_generator::streaming::ChunkType::Css => "css".to_string(),
                            arkavo_ui_generator::streaming::ChunkType::JavaScript => {
                                "js".to_string()
                            }
                        },
                        content: chunk.content,
                        done: chunk.done,
                    };

                    if let Err(e) = tx_clone.send(stream_event).await {
                        eprintln!("AG-UI: Failed to send stream event: {}", e);
                        break;
                    }

                    if chunk.done {
                        println!(
                            "AG-UI: Stream complete for part {} after {} chunks",
                            part_id_clone, chunk_count
                        );
                        let version_id = uuid::Uuid::new_v4().to_string();
                        let applied_event = AgUiEvent::AppliedPart {
                            part_id: part_id_clone.clone(),
                            version_id,
                        };
                        if let Err(e) = tx_clone.send(applied_event).await {
                            eprintln!("AG-UI: Failed to send applied event: {}", e);
                        }
                        break;
                    }
                }
                println!(
                    "AG-UI: Stream receiver task ended for part: {}",
                    part_id_clone
                );
            });
        }

        AgUiEvent::RequestTaskList => {
            println!("AG-UI: Received RequestTaskList");

            // Get tasks from connection info or return empty list
            let conn_guard = connections.read().await;
            let tasks: Vec<crate::types::TaskInfo> =
                if let Some(conn_info) = conn_guard.get(session_id) {
                    // Get tasks associated with this session
                    conn_info
                        .subscriptions
                        .iter()
                        .filter_map(|_| None)
                        .collect()
                } else {
                    Vec::new()
                };

            let task_list = AgUiEvent::TaskList {
                tasks,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            tx.send(task_list).await?;
            println!("AG-UI: Sent TaskList");
        }

        AgUiEvent::SubmitTask {
            description,
            target_agent: _,
        } => {
            println!("AG-UI: Received SubmitTask: {}", description);

            let task_id = uuid::Uuid::new_v4().to_string();

            // All human tasks route to orchestrator for decomposition
            // The orchestrator will break down the task and delegate to specialists
            let agents_list = agents.read().await;
            let orchestrator = agents_list.iter().find(|a| {
                let purpose = a.get("purpose").and_then(|v| v.as_str()).unwrap_or("");
                purpose.to_lowercase().contains("orchestrat")
            });

            if let Some(orch) = orchestrator {
                if let Some(endpoint) = orch.get("endpoint").and_then(|v| v.as_str()) {
                    let orch_id = orch
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("orchestrator");
                    println!(
                        "AG-UI: Routing task to orchestrator {} at {}",
                        orch_id, endpoint
                    );
                }
            } else {
                println!("AG-UI: No orchestrator found, task queued for when one connects");
            }

            // Send confirmation back to client
            let task_submitted = AgUiEvent::TaskSubmitted {
                task_id: task_id.clone(),
                status: "submitted".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            tx.send(task_submitted).await?;

            // Also send task status changed to update the list
            let status_changed = AgUiEvent::TaskStatusChanged {
                task_id,
                status: "submitted".to_string(),
                progress: Some(0.0),
                result: None,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };

            // Broadcast to all connections
            let conns = connections.read().await;
            for (_, conn_info) in conns.iter() {
                let _ = conn_info._ws_tx.send(status_changed.clone()).await;
            }
        }

        _ => {
            println!("AG-UI: Received event: {event:?}");
        }
    }

    Ok(())
}

async fn handle_agent_proxy(
    agent_id: String,
    body: serde_json::Value,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
) -> Json<serde_json::Value> {
    // Find the agent
    let agents_list = agents.read().await;
    let agent = agents_list
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(&agent_id));

    if let Some(agent_info) = agent {
        if let Some(endpoint) = agent_info.get("endpoint").and_then(|v| v.as_str()) {
            // Forward the request to the agent
            let request_id = body.get("id").cloned();
            match forward_to_agent(endpoint, body).await {
                Ok(response) => Json(response),
                Err(e) => {
                    let error_response = serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": {
                            "code": -32603,
                            "message": format!("Failed to forward request: {}", e)
                        },
                        "id": request_id
                    });
                    Json(error_response)
                }
            }
        } else {
            let error_response = serde_json::json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32602,
                    "message": "Agent endpoint not found"
                },
                "id": body.get("id").cloned()
            });
            Json(error_response)
        }
    } else {
        let error_response = serde_json::json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -32602,
                "message": format!("Agent {agent_id} not found")
            },
            "id": body.get("id").cloned()
        });
        Json(error_response)
    }
}

async fn forward_to_agent(
    endpoint: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://{endpoint}");

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let json_response = response.json::<serde_json::Value>().await?;
    Ok(json_response)
}

async fn handle_telemetry_websocket(
    mut ws: WebSocket,
    telemetry_rx: Arc<RwLock<mpsc::Receiver<TelemetryEvent>>>,
) {
    println!("New telemetry WebSocket connection");

    // Take ownership of the receiver
    let mut rx = telemetry_rx.write().await;

    // Forward telemetry events to WebSocket
    while let Some(event) = rx.recv().await {
        if let Ok(json) = serde_json::to_string(&event)
            && ws.send(Message::Text(json)).await.is_err()
        {
            break;
        }
    }

    println!("Telemetry WebSocket connection closed");
}

async fn handle_debug_websocket(mut ws: WebSocket, debug_handler: Option<Arc<DebugHandler>>) {
    if let Some(handler) = debug_handler {
        println!("New debug WebSocket connection");

        // Create channel for events
        let (tx, mut rx) = mpsc::channel::<AgUiEvent>(100);

        // Spawn task to forward events
        let ws_clone = Arc::new(tokio::sync::Mutex::new(ws));
        let ws_for_forward = ws_clone.clone();
        let forward_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Ok(json) = serde_json::to_string(&event) {
                    let mut ws_guard = ws_for_forward.lock().await;
                    if ws_guard.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }
        });

        // Handle incoming debug commands
        let ws_for_recv = ws_clone.clone();
        loop {
            let msg = {
                let mut ws_guard = ws_for_recv.lock().await;
                ws_guard.recv().await
            };

            match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(cmd) = serde_json::from_str::<DebugCommand>(&text) {
                        match cmd {
                            DebugCommand::SubscribeSession { session_id } => {
                                let _ = handler.subscribe_to_session(session_id, tx.clone()).await;
                            }
                            DebugCommand::UnsubscribeSession { session_id } => {
                                handler.unsubscribe_session(&session_id).await;
                                let event = AgUiEvent::StateDelta {
                                    patch: vec![crate::types::JsonPatch::Add {
                                        path: "/unsubscribed".to_string(),
                                        value: serde_json::Value::String(session_id.clone()),
                                    }],
                                    event_id: format!("unsubscribed-{session_id}"),
                                };
                                let _ = tx.send(event).await;
                            }
                            DebugCommand::GetRecentSessions { limit } => {
                                if let Ok(sessions) = handler.get_recent_sessions(limit).await {
                                    let event = AgUiEvent::StateDelta {
                                        patch: vec![crate::types::JsonPatch::Add {
                                            path: "/sessions".to_string(),
                                            value: serde_json::to_value(sessions)
                                                .unwrap_or(serde_json::Value::Null),
                                        }],
                                        event_id: format!("sessions-{}", uuid::Uuid::new_v4()),
                                    };
                                    let _ = tx.send(event).await;
                                }
                            }
                            DebugCommand::GetActiveSessions => {
                                let sessions = handler.get_active_sessions().await;
                                let event = AgUiEvent::StateDelta {
                                    patch: vec![crate::types::JsonPatch::Add {
                                        path: "/active_sessions".to_string(),
                                        value: serde_json::to_value(sessions)
                                            .unwrap_or(serde_json::Value::Null),
                                    }],
                                    event_id: format!("active-sessions-{}", uuid::Uuid::new_v4()),
                                };
                                let _ = tx.send(event).await;
                            }
                            DebugCommand::AttachToAgent { agent_id } => {
                                let session_id = uuid::Uuid::new_v4().to_string();
                                handler.attach_to_agent(agent_id.clone(), session_id).await;
                                let event = AgUiEvent::StateDelta {
                                    patch: vec![crate::types::JsonPatch::Add {
                                        path: "/attached_agent".to_string(),
                                        value: serde_json::Value::String(agent_id),
                                    }],
                                    event_id: format!("attached-{}", uuid::Uuid::new_v4()),
                                };
                                let _ = tx.send(event).await;
                            }
                            DebugCommand::DetachFromAgent { agent_id } => {
                                handler.detach_from_agent(&agent_id).await;
                                let event = AgUiEvent::StateDelta {
                                    patch: vec![crate::types::JsonPatch::Remove {
                                        path: format!("/attached_agent/{agent_id}"),
                                    }],
                                    event_id: format!("detached-{}", uuid::Uuid::new_v4()),
                                };
                                let _ = tx.send(event).await;
                            }
                            DebugCommand::StartRecording { session_id } => {
                                handler.start_recording(session_id.clone()).await;
                                let event = AgUiEvent::StateDelta {
                                    patch: vec![crate::types::JsonPatch::Add {
                                        path: "/recording".to_string(),
                                        value: serde_json::json!({ "session_id": session_id, "status": "started" }),
                                    }],
                                    event_id: format!("recording-started-{}", uuid::Uuid::new_v4()),
                                };
                                let _ = tx.send(event).await;
                            }
                            DebugCommand::StopRecording { session_id } => {
                                handler.stop_recording(&session_id).await;
                                let event = AgUiEvent::StateDelta {
                                    patch: vec![crate::types::JsonPatch::Add {
                                        path: "/recording".to_string(),
                                        value: serde_json::json!({ "session_id": session_id, "status": "stopped" }),
                                    }],
                                    event_id: format!("recording-stopped-{}", uuid::Uuid::new_v4()),
                                };
                                let _ = tx.send(event).await;
                            }
                            DebugCommand::GetSessionEvents { session_id, limit } => {
                                match handler.get_session_events(&session_id, limit).await {
                                    Ok(events) => {
                                        let event = AgUiEvent::StateDelta {
                                            patch: vec![crate::types::JsonPatch::Add {
                                                path: "/session_events".to_string(),
                                                value: serde_json::json!({
                                                    "session_id": session_id,
                                                    "events": events,
                                                    "count": events.len()
                                                }),
                                            }],
                                            event_id: format!("events-{}", uuid::Uuid::new_v4()),
                                        };
                                        let _ = tx.send(event).await;
                                    }
                                    Err(e) => {
                                        let error = AgUiEvent::Error {
                                            code: "EVENT_FETCH_FAILED".to_string(),
                                            message: format!("Failed to fetch events: {e}"),
                                        };
                                        let _ = tx.send(error).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                _ => {}
            }
        }

        forward_task.abort();
        println!("Debug WebSocket connection closed");
    } else {
        // Debug handler not available
        let error = AgUiEvent::Error {
            code: "DEBUG_DISABLED".to_string(),
            message: "Debug handler not configured".to_string(),
        };
        if let Ok(json) = serde_json::to_string(&error) {
            let _ = ws.send(Message::Text(json)).await;
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum DebugCommand {
    SubscribeSession {
        session_id: String,
    },
    UnsubscribeSession {
        session_id: String,
    },
    GetRecentSessions {
        limit: usize,
    },
    GetActiveSessions,
    AttachToAgent {
        agent_id: String,
    },
    DetachFromAgent {
        agent_id: String,
    },
    StartRecording {
        session_id: String,
    },
    StopRecording {
        session_id: String,
    },
    GetSessionEvents {
        session_id: String,
        limit: Option<usize>,
    },
}
