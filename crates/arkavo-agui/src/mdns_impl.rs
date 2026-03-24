//! mDNS implementation using pure Rust mdns-sd crate

#[cfg(feature = "mdns")]
pub mod mdns {
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{RwLock, mpsc};

    /// Discovers A2A agents using mDNS
    pub async fn discover_agents(
        agents: Arc<RwLock<Vec<serde_json::Value>>>,
        agent_connections: Arc<
            RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>,
        >,
        telemetry_tx: mpsc::Sender<crate::agent_connection::TelemetryEvent>,
        browser_connections: Arc<RwLock<HashMap<String, crate::gateway::ConnectionInfo>>>,
        security_handler: Arc<RwLock<crate::security_handler::SecurityHandler>>,
        context_topology_cache: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("AG-UI: mDNS daemon starting...");

        let mdns = ServiceDaemon::new()?;
        let service_type = "_a2a._tcp.local.";
        let receiver = mdns.browse(service_type)?;
        println!("AG-UI: mDNS browsing for {service_type}");

        // Channel bridges blocking mDNS recv thread → async handler
        let (info_tx, mut info_rx) = mpsc::channel::<ServiceInfo>(16);

        // Blocking thread: receive mDNS events, forward resolved services
        tokio::task::spawn_blocking(move || {
            loop {
                if let Ok(event) = receiver.recv_timeout(Duration::from_secs(5)) {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            println!("AG-UI: mDNS ServiceResolved: {}", info.get_fullname());
                            if info_tx.blocking_send(info).is_err() {
                                break; // Receiver dropped
                            }
                        }
                        ServiceEvent::ServiceFound(_, fullname) => {
                            println!("AG-UI: mDNS ServiceFound: {fullname}");
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            println!("AG-UI: mDNS ServiceRemoved: {fullname}");
                        }
                        ServiceEvent::SearchStarted(stype) => {
                            println!("AG-UI: mDNS SearchStarted: {stype}");
                        }
                        ServiceEvent::SearchStopped(stype) => {
                            println!("AG-UI: mDNS SearchStopped: {stype}");
                        }
                        _ => {}
                    }
                }
            }
        });

        // Async task: handle discovered services without blocking
        tokio::spawn(async move {
            while let Some(info) = info_rx.recv().await {
                handle_service_discovered(
                    info,
                    agents.clone(),
                    agent_connections.clone(),
                    telemetry_tx.clone(),
                    browser_connections.clone(),
                    security_handler.clone(),
                    context_topology_cache.clone(),
                )
                .await;
            }
        });

        // Keep the daemon alive (it's dropped when this future completes)
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }

    async fn handle_service_discovered(
        info: ServiceInfo,
        agents: Arc<RwLock<Vec<serde_json::Value>>>,
        agent_connections: Arc<
            RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>,
        >,
        telemetry_tx: mpsc::Sender<crate::agent_connection::TelemetryEvent>,
        browser_connections: Arc<RwLock<HashMap<String, crate::gateway::ConnectionInfo>>>,
        security_handler: Arc<RwLock<crate::security_handler::SecurityHandler>>,
        context_topology_cache: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    ) {
        let service_name = info.get_fullname();
        let port = info.get_port();

        // Get the first IP address
        let host = info
            .get_addresses()
            .iter()
            .next()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        println!(
            "AG-UI: Discovered service: {} at {}:{}",
            service_name, host, port
        );

        // Extract agent information from properties
        let properties = info.get_properties();
        let mut agent_id = service_name.to_string();
        if agent_id.starts_with("arkavo-agent-") {
            agent_id = agent_id.trim_start_matches("arkavo-agent-").to_string();
        }

        let purpose = properties
            .get("purpose")
            .map(|v| v.val_str().to_string())
            .unwrap_or_else(|| "Agent discovered via mDNS".to_string());

        let model = properties
            .get("model")
            .map(|v| v.val_str().to_string())
            .unwrap_or_else(|| "auto (router-selected)".to_string());

        // Extract agent_id from properties if available
        if let Some(id_prop) = properties.get("agent_id") {
            agent_id = id_prop.val_str().to_string();
        }

        // Use IP from properties if host is 0.0.0.0
        let final_host = if host == "0.0.0.0" {
            if let Some(ip_prop) = properties.get("ip") {
                ip_prop.val_str().to_string()
            } else {
                println!(
                    "AG-UI: Service advertised 0.0.0.0 with no IP in TXT records, using 127.0.0.1"
                );
                "127.0.0.1".to_string()
            }
        } else {
            host
        };

        let agent_info = serde_json::json!({
            "id": agent_id,
            "name": agent_id,
            "purpose": purpose,
            "model": model,
            "endpoint": format!("{}:{}", final_host, port)
        });

        // Add to agents list
        let mut agents_list = agents.write().await;

        // Check if agent already exists
        let exists = agents_list
            .iter()
            .any(|a| a.get("id") == agent_info.get("id"));

        if !exists {
            println!("AG-UI: Adding new agent to list: {}", agent_id);

            // Auto-connect to discovered agent
            let agent_id_clone = agent_id.clone();
            let endpoint = format!("{}:{}", final_host, port);
            let telemetry_tx_clone = telemetry_tx.clone();
            let agent_connections_clone = agent_connections.clone();
            tokio::spawn(async move {
                println!(
                    "AG-UI: Auto-connecting to agent: {} at {}",
                    agent_id_clone, endpoint
                );

                let connection = Arc::new(crate::agent_connection::AgentConnection::new(
                    agent_id_clone.clone(),
                    endpoint.clone(),
                    telemetry_tx_clone,
                ));

                if let Err(e) = connection.connect().await {
                    println!(
                        "AG-UI: Failed to connect to agent {}: {}",
                        agent_id_clone, e
                    );
                } else {
                    println!("AG-UI: Connected to agent: {}", agent_id_clone);

                    // Subscribe to push-based metrics stream
                    let (metrics_tx, mut metrics_rx) = mpsc::channel::<crate::types::AgUiEvent>(32);
                    if let Err(e) = connection
                        .subscribe_metrics(metrics_tx, security_handler.clone())
                        .await
                    {
                        println!(
                            "AG-UI: Metrics subscription failed for {}: {} (falling back to polling)",
                            agent_id_clone, e
                        );
                    } else {
                        println!("AG-UI: Metrics subscription active for {}", agent_id_clone);
                        // Forward metrics events to all browser sessions
                        let browser_conns = browser_connections.clone();
                        let topo_cache = context_topology_cache.clone();
                        let cache_agent_id = agent_id_clone.clone();
                        tokio::spawn(async move {
                            while let Some(event) = metrics_rx.recv().await {
                                // Cache context topology telemetry for aggregation
                                if let crate::types::AgUiEvent::TelemetryEvent {
                                    ref event_type,
                                    ref details,
                                    ..
                                } = event
                                    && event_type == "context_topology"
                                {
                                    topo_cache
                                        .write()
                                        .await
                                        .insert(cache_agent_id.clone(), details.clone());
                                }
                                let conns = browser_conns.read().await;
                                for (_, ci) in conns.iter() {
                                    let _ = ci._ws_tx.send(event.clone()).await;
                                }
                            }
                        });
                    }

                    let mut connections = agent_connections_clone.write().await;
                    connections.insert(agent_id_clone.clone(), connection);
                }
            });

            agents_list.push(agent_info);
        } else if let Some(existing) = agents_list
            .iter_mut()
            .find(|a| a.get("id") == agent_info.get("id"))
        {
            // Update existing agent with fresh mDNS data (e.g. model change)
            *existing = agent_info;
        }
    }

    /// Registers an A2A agent as an mDNS service
    pub fn register_service(
        agent_id: &str,
        port: u16,
        purpose: &str,
        model: &str,
    ) -> Result<ServiceDaemon, Box<dyn std::error::Error>> {
        println!("AG-UI: Registering mDNS service for agent: {}", agent_id);

        let mdns = ServiceDaemon::new()?;

        // Create service info
        let service_type = "_a2a._tcp.local.";
        let instance_name = format!("arkavo-agent-{}", agent_id);
        let host_ipv4 = "0.0.0.0"; // Will be resolved to actual IP
        let host_name = format!("{}.local.", agent_id);

        let mut properties = HashMap::new();
        properties.insert("agent_id".to_string(), agent_id.to_string());
        properties.insert("purpose".to_string(), purpose.to_string());
        properties.insert("model".to_string(), model.to_string());

        // Get local IP if possible
        // Note: mdns-sd will handle IP discovery internally

        let service_info = ServiceInfo::new(
            service_type,
            &instance_name,
            &host_name,
            host_ipv4,
            port,
            properties,
        )?;

        mdns.register(service_info)?;

        println!("AG-UI: mDNS service registered successfully");
        Ok(mdns)
    }
}

#[cfg(not(feature = "mdns"))]
pub mod mdns {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{RwLock, mpsc};

    pub async fn discover_agents(
        _agents: Arc<RwLock<Vec<serde_json::Value>>>,
        _agent_connections: Arc<
            RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>,
        >,
        _telemetry_tx: mpsc::Sender<crate::agent_connection::TelemetryEvent>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Err("mDNS feature not compiled in".into())
    }

    pub fn register_service(
        _agent_id: &str,
        _port: u16,
        _purpose: &str,
        _model: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Err("mDNS feature not compiled in".into())
    }
}
