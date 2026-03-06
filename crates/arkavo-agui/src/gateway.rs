use crate::agent_connection::{AgentConnection, TelemetryEvent};
use crate::budget_handler::BudgetHandler;
use crate::cost_handler::CostHandler;
use crate::dataflow_handler::DataflowHandler;
use crate::debug_handler::DebugHandler;
use crate::security_handler::SecurityHandler;
use crate::types::*;
use arkavo_observability::metrics_snapshot::{MetricsSampler, MetricsSamplerConfig};
use arkavo_protocol::rate_limit::{IpRateLimiter, RateLimitConfig};
use arkavo_router::learning::LearningModule;
use axum::{
    Router, middleware,
    routing::{get, post},
};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub struct ConnectionInfo {
    pub _ws_tx: mpsc::Sender<AgUiEvent>,
    pub _agent_id: Option<String>,
    pub subscriptions: Vec<SubscriptionHandle>,
    pub current_plan: Option<Vec<arkavo_ui_generator::planner::ComponentPart>>,
    pub current_prompt: Option<String>,
}

/// Tracks a task submitted via the UI and delegated to an agent
#[derive(Debug, Clone)]
pub struct TrackedTask {
    pub id: String,
    pub description: String,
    pub target_agent: Option<String>,
    pub status: String,
    pub progress: Option<f32>,
    pub result: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub task_category: Option<String>,
    pub first_working_at: Option<String>,
    /// "ui" for user-submitted, "agent" for autonomous agent tasks
    pub source: String,
}

#[derive(Clone)]
pub struct AppState {
    pub connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    pub agents: Arc<RwLock<Vec<serde_json::Value>>>,
    pub agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    pub budget_handler: Arc<RwLock<BudgetHandler>>,
    pub cost_handler: Arc<RwLock<CostHandler>>,
    pub security_handler: Arc<RwLock<SecurityHandler>>,
    pub initial_prompt: Arc<RwLock<Option<String>>>,
    pub dataflow_handler: Arc<DataflowHandler>,
    pub debug_handler: Option<Arc<DebugHandler>>,
    pub rate_limiter: Arc<IpRateLimiter>,
    pub task_store: Arc<RwLock<HashMap<String, TrackedTask>>>,
    pub learning_module: Arc<RwLock<LearningModule>>,
    pub routing_history: Arc<RwLock<VecDeque<RoutingRecord>>>,
    pub lesson_tx: Option<mpsc::Sender<arkavo_router::learning::Lesson>>,
    pub lesson_store: Arc<RwLock<Vec<arkavo_router::learning::Lesson>>>,
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
    security_handler: Arc<RwLock<SecurityHandler>>,
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
            security_handler: Arc::new(RwLock::new(SecurityHandler::new())),
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

        // Initialize security handler from AGENTS.md config
        {
            let mut sec = self.security_handler.write().await;
            sec.configure_from_agents_md();
        }

        // Initialize budget handler with AGENTS.md config
        {
            let budget_config = load_budget_config_from_agents_md();
            let (budget_tx, mut budget_rx) = mpsc::channel::<AgUiEvent>(100);
            let mut handler = self.budget_handler.write().await;
            handler.initialize(budget_config, budget_tx).await?;

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

        // Start mDNS discovery
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

        // Start metrics sampler
        let telemetry_tx_for_metrics = self.telemetry_tx.clone();
        tokio::spawn(async move {
            let config = MetricsSamplerConfig::default();
            let mut sampler = MetricsSampler::new_with_name(config, "arkavo-agui".to_string());
            let metrics_collector = arkavo_observability::metrics::MetricsCollector::new();
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let snapshot = sampler.sample_metrics(&metrics_collector);
                let _ = telemetry_tx_for_metrics
                    .send(TelemetryEvent::MetricsSnapshot { snapshot })
                    .await;
            }
        });

        // Register health reporters
        {
            use arkavo_observability::health_reporter::HealthRegistry;
            let registry = HealthRegistry::global();
            registry
                .register(Arc::new(crate::health::AguiHealthReporter::new()))
                .await;
            use arkavo_router::health::RouterHealthReporter;
            let cc = Arc::new(arkavo_router::ConnectivityChecker::new());
            registry
                .register(Arc::new(RouterHealthReporter::new(cc)))
                .await;
            use arkavo_ui_generator::health::GlobalHealthReporterWrapper;
            registry
                .register(Arc::new(GlobalHealthReporterWrapper))
                .await;
            println!("AG-UI: Health reporters registered");
        }

        let (health_alert_tx, mut health_alert_rx) = mpsc::channel::<AgUiEvent>(100);

        // Start health monitor
        {
            use arkavo_mcp_tools::registry::ToolRegistry;
            use arkavo_memory::MemoryStorage;
            let storage = Arc::new(MemoryStorage::new().await?);
            let tool_registry = Arc::new(ToolRegistry::new(storage));
            let health_monitor = crate::health_monitor::HealthMonitor::new(tool_registry)
                .await?
                .with_interval(30);
            let _health_task = health_monitor.start(health_alert_tx.clone()).await?;
            println!("AG-UI: Health monitor started (30s interval)");
        }

        // Start command health collector
        {
            use crate::command_health_collector::CommandHealthCollector;
            use crate::timeout_handler::TimeoutHandler;
            let (command_health_tx, command_health_rx) = mpsc::channel(100);
            let collector = CommandHealthCollector::new(30);
            let _collector_task = collector.start(command_health_tx).await?;
            let timeout_handler = TimeoutHandler::new().await?;
            let _timeout_task = timeout_handler
                .start(command_health_rx, health_alert_tx.clone())
                .await;
            println!("AG-UI: Command health collector started");
        }

        // Broadcast health alerts
        let connections_for_health = self.connections.clone();
        tokio::spawn(async move {
            while let Some(alert_event) = health_alert_rx.recv().await {
                let conns = connections_for_health.read().await;
                for (_, conn_info) in conns.iter() {
                    let _ = conn_info._ws_tx.send(alert_event.clone()).await;
                }
            }
        });

        // Push periodic status updates every 30s
        crate::gateway_monitors::spawn_status_broadcaster(self.connections.clone());

        // Monitor agent changes
        crate::gateway_monitors::spawn_agent_monitor(
            self.connections.clone(),
            discovered_agents.clone(),
        );

        // Bridge TelemetryEvent → AgUiEvent on the main WebSocket
        let telemetry_rx = self
            .telemetry_rx
            .take()
            .expect("telemetry_rx already taken");
        let connections_for_telemetry = self.connections.clone();
        tokio::spawn(async move {
            let mut rx = telemetry_rx;
            while let Some(event) = rx.recv().await {
                let ui_event = match &event {
                    TelemetryEvent::MessageRouted {
                        agent_id,
                        message_type,
                        direction,
                        timestamp,
                        ..
                    } => {
                        let dir = match direction {
                            crate::agent_connection::MessageDirection::Inbound => "inbound",
                            crate::agent_connection::MessageDirection::Outbound => "outbound",
                        };
                        AgUiEvent::A2AMessage {
                            from_agent: if dir == "outbound" {
                                "gateway".to_string()
                            } else {
                                agent_id.clone()
                            },
                            to_agent: if dir == "outbound" {
                                agent_id.clone()
                            } else {
                                "gateway".to_string()
                            },
                            method: message_type.clone(),
                            direction: dir.to_string(),
                            timestamp: timestamp.to_rfc3339(),
                        }
                    }
                    TelemetryEvent::MetricsSnapshot { snapshot } => {
                        use std::sync::atomic::{AtomicU64, Ordering};
                        static SNAPSHOT_COUNTER: AtomicU64 = AtomicU64::new(0);
                        let count = SNAPSHOT_COUNTER.fetch_add(1, Ordering::Relaxed);
                        if !count.is_multiple_of(30) {
                            continue;
                        }
                        AgUiEvent::TelemetryEvent {
                            event_type: "metrics_snapshot".to_string(),
                            agent_id: "system".to_string(),
                            details: serde_json::to_value(snapshot)
                                .unwrap_or(serde_json::Value::Null),
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        }
                    }
                    _ => {
                        let (event_type, agent_id, timestamp) = match &event {
                            TelemetryEvent::AgentConnected {
                                agent_id,
                                timestamp,
                                ..
                            } => ("agent_connected", agent_id.as_str(), *timestamp),
                            TelemetryEvent::AgentDisconnected {
                                agent_id,
                                timestamp,
                                ..
                            } => ("agent_disconnected", agent_id.as_str(), *timestamp),
                            TelemetryEvent::AgentReconnecting {
                                agent_id,
                                timestamp,
                                ..
                            } => ("agent_reconnecting", agent_id.as_str(), *timestamp),
                            TelemetryEvent::ToolCallExecuted {
                                agent_id,
                                timestamp,
                                ..
                            } => ("tool_call_executed", agent_id.as_str(), *timestamp),
                            _ => continue,
                        };
                        let details =
                            serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
                        AgUiEvent::TelemetryEvent {
                            event_type: event_type.to_string(),
                            agent_id: agent_id.to_string(),
                            details,
                            timestamp: timestamp.to_rfc3339(),
                        }
                    }
                };
                let conns = connections_for_telemetry.read().await;
                for (_, conn_info) in conns.iter() {
                    let _ = conn_info._ws_tx.send(ui_event.clone()).await;
                }
            }
        });

        let rate_limiter = Arc::new(IpRateLimiter::new(RateLimitConfig::default()));
        // Wire budget manager into cost handler for ROI dashboard
        let cost_handler = {
            let mut handler = CostHandler::new();
            let budget_guard = self.budget_handler.read().await;
            if let Some(manager) = budget_guard.manager() {
                handler.set_budget_manager(manager);
            }
            Arc::new(RwLock::new(handler))
        };

        // Create lesson pipeline: extracted lessons flow into a local store for guidance injection
        let lesson_store: Arc<RwLock<Vec<arkavo_router::learning::Lesson>>> =
            Arc::new(RwLock::new(Vec::new()));
        let (lesson_tx, mut lesson_rx) = mpsc::channel::<arkavo_router::learning::Lesson>(64);

        let lesson_store_for_rx = lesson_store.clone();
        tokio::spawn(async move {
            const MAX_LESSONS_PER_KEY: usize = 5;
            while let Some(lesson) = lesson_rx.recv().await {
                println!(
                    "AG-UI: Caching lesson for {} on {}: {}",
                    lesson.agent_id, lesson.category, lesson.pattern.condition
                );
                let mut store = lesson_store_for_rx.write().await;
                store.push(lesson);
                // Evict oldest when too many for same (agent, category) key
                let len = store.len();
                if len > MAX_LESSONS_PER_KEY * 10 {
                    store.drain(..len - MAX_LESSONS_PER_KEY * 10);
                }
            }
        });

        let state = AppState {
            connections: self.connections.clone(),
            agents: discovered_agents.clone(),
            agent_connections: self.agent_connections.clone(),
            budget_handler: self.budget_handler.clone(),
            cost_handler,
            security_handler: self.security_handler.clone(),
            initial_prompt: Arc::new(RwLock::new(self.initial_prompt.clone())),
            dataflow_handler: self.dataflow_handler.clone(),
            debug_handler: self.debug_handler.clone(),
            rate_limiter: rate_limiter.clone(),
            task_store: Arc::new(RwLock::new(HashMap::new())),
            learning_module: Arc::new(RwLock::new(LearningModule::new())),
            routing_history: Arc::new(RwLock::new(VecDeque::new())),
            lesson_tx: Some(lesson_tx),
            lesson_store,
        };

        // Sync agent-internal HRM tasks into the UI dashboard
        crate::gateway_task_sync::spawn_agent_task_sync(
            state.connections.clone(),
            state.agent_connections.clone(),
            state.task_store.clone(),
        );

        // Rate-limited API routes
        let api_routes = Router::new()
            .route("/ws", get(crate::gateway_ws::websocket_handler))
            .route(
                "/agent/:id",
                post(crate::gateway_proxy::agent_proxy_handler),
            )
            .route(
                "/api/dataflow/*path",
                post(crate::gateway_proxy::dataflow_handler),
            )
            .route(
                "/debug",
                get(crate::gateway_status::debug_websocket_handler),
            )
            .layer(middleware::from_fn(
                arkavo_protocol::ip_rate_limit_middleware,
            ))
            .layer(axum::Extension(rate_limiter));

        // Static file routes (no rate limiting — browser loads many files at once)
        let static_routes = Router::new()
            .route("/", get(crate::gateway_static::index_handler))
            .route(
                "/static/*path",
                get(crate::gateway_static::static_file_handler),
            );

        let app = static_routes
            .merge(api_routes)
            .layer(crate::gateway_security::security_headers())
            .with_state(state);

        let addr: SocketAddr = ([0, 0, 0, 0], self.port).into();
        println!("Starting AG-UI Gateway on http://127.0.0.1:{}", self.port);
        println!("Open http://127.0.0.1:{} in your web browser", self.port);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        Ok(())
    }
}

/// Load budget config from AGENTS.md, falling back to defaults
fn load_budget_config_from_agents_md() -> arkavo_budget::BudgetConfig {
    let agent_config = arkavo_router::load_agent_config().unwrap_or_default();

    match agent_config.budget {
        Some(ref budget_yaml) => {
            let mut config = arkavo_budget::BudgetConfig::default();
            if let Some(session_cost) = budget_yaml.max_cost_per_session {
                config.limits.session_limit =
                    Some(arkavo_budget::TokenCost::from_dollars(session_cost));
            }
            if let Some(daily_cost) = budget_yaml.max_cost_per_day {
                config.limits.daily_limit =
                    Some(arkavo_budget::TokenCost::from_dollars(daily_cost));
            }
            tracing::info!(
                "AG-UI: Budget config loaded from AGENTS.md (session={:?}, daily={:?})",
                budget_yaml.max_cost_per_session,
                budget_yaml.max_cost_per_day
            );
            config
        }
        None => arkavo_budget::BudgetConfig::default(),
    }
}
