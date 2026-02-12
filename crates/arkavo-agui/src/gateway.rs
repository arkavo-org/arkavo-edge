use crate::agent_connection::{AgentConnection, TelemetryEvent};
use crate::budget_handler::BudgetHandler;
use crate::cost_handler::CostHandler;
use crate::dataflow_handler::DataflowHandler;
use crate::debug_handler::DebugHandler;
use crate::types::*;
use arkavo_observability::metrics_snapshot::{MetricsSampler, MetricsSamplerConfig};
use axum::{
    Router,
    routing::{get, post},
};
use std::collections::HashMap;
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

#[derive(Clone)]
pub struct AppState {
    pub connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    pub agents: Arc<RwLock<Vec<serde_json::Value>>>,
    pub agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    pub budget_handler: Arc<RwLock<BudgetHandler>>,
    pub cost_handler: Arc<RwLock<CostHandler>>,
    pub initial_prompt: Arc<RwLock<Option<String>>>,
    pub dataflow_handler: Arc<DataflowHandler>,
    pub telemetry_rx: Arc<RwLock<mpsc::Receiver<TelemetryEvent>>>,
    pub debug_handler: Option<Arc<DebugHandler>>,
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

        // Initialize budget handler
        {
            let budget_config = arkavo_budget::BudgetConfig::default();
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
        spawn_status_broadcaster(self.connections.clone());

        // Monitor agent changes
        spawn_agent_monitor(self.connections.clone(), discovered_agents.clone());

        let telemetry_rx = self
            .telemetry_rx
            .take()
            .expect("telemetry_rx already taken");

        let state = AppState {
            connections: self.connections.clone(),
            agents: discovered_agents.clone(),
            agent_connections: self.agent_connections.clone(),
            budget_handler: self.budget_handler.clone(),
            cost_handler: Arc::new(RwLock::new(CostHandler::new())),
            initial_prompt: Arc::new(RwLock::new(self.initial_prompt.clone())),
            dataflow_handler: self.dataflow_handler.clone(),
            telemetry_rx: Arc::new(RwLock::new(telemetry_rx)),
            debug_handler: self.debug_handler.clone(),
        };

        let app = Router::new()
            .route("/", get(crate::gateway_static::index_handler))
            .route(
                "/static/*path",
                get(crate::gateway_static::static_file_handler),
            )
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
                "/telemetry",
                get(crate::gateway_status::telemetry_websocket_handler),
            )
            .route(
                "/debug",
                get(crate::gateway_status::debug_websocket_handler),
            )
            .layer(crate::gateway_security::security_headers())
            .with_state(state);

        let addr: SocketAddr = ([0, 0, 0, 0], self.port).into();
        println!("Starting AG-UI Gateway on http://127.0.0.1:{}", self.port);
        println!("Open http://127.0.0.1:{} in your web browser", self.port);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

fn spawn_status_broadcaster(connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            use arkavo_mcp_tools::registry::ToolRegistry;
            use arkavo_memory::MemoryStorage;
            use arkavo_observability::health_reporter::HealthRegistry;
            use arkavo_router::Router as LlmRouter;

            let storage = match MemoryStorage::new().await {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    eprintln!("Failed to init storage for health: {e}");
                    continue;
                }
            };
            let registry = ToolRegistry::new(storage);
            let tools = registry.list_tools();
            let browser_tool = tools.iter().find(|t| t.name.contains("browser"));
            let health_registry = HealthRegistry::global();
            let health_reports = health_registry.check_all().await;
            let overall_health = health_registry.get_overall_status().await;
            let conns = connections.read().await;

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

            if let Ok(router) = LlmRouter::new().await {
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
                for (_, conn_info) in conns.iter() {
                    let _ = conn_info._ws_tx.send(status_event.clone()).await;
                }
            }
        }
    });
}

fn spawn_agent_monitor(
    connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
) {
    tokio::spawn(async move {
        let mut previous: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            let agents_list = agents.read().await;
            let current: std::collections::HashSet<String> = agents_list
                .iter()
                .filter_map(|a| a.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .collect();

            for agent_id in current.difference(&previous) {
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
                    let conns = connections.read().await;
                    for (_, ci) in conns.iter() {
                        let _ = ci._ws_tx.send(event.clone()).await;
                    }
                }
            }

            for agent_id in previous.difference(&current) {
                let event = AgUiEvent::AgentLost {
                    agent_id: agent_id.clone(),
                    reason: "Agent no longer discovered".to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                let conns = connections.read().await;
                for (_, ci) in conns.iter() {
                    let _ = ci._ws_tx.send(event.clone()).await;
                }
            }

            previous = current;
        }
    });
}
