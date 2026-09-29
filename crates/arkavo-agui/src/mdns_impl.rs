//! mDNS implementation using pure Rust mdns-sd crate

#[cfg(feature = "mdns")]
pub mod mdns {
    use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{RwLock, mpsc};

    /// An mDNS daemon that also listens on the loopback interfaces.
    ///
    /// An agent started with `--bind 127.0.0.1`, or whose kit names a
    /// loopback address, listens on loopback, and the record of such an agent is
    /// announced on the loopback interface only. mdns-sd leaves that
    /// interface out unless asked, which hides every such agent from a
    /// browser on the same machine.
    pub fn browsing_daemon() -> mdns_sd::Result<ServiceDaemon> {
        let daemon = ServiceDaemon::new()?;
        daemon.enable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])?;
        Ok(daemon)
    }

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

        let mdns = browsing_daemon()?;
        let service_type = "_a2a._tcp.local.";
        let receiver = mdns.browse(service_type)?;
        println!("AG-UI: mDNS browsing for {service_type}");

        // Channels bridge blocking mDNS recv thread → async handlers
        let (info_tx, mut info_rx) = mpsc::channel::<ServiceInfo>(16);
        let (remove_tx, mut remove_rx) = mpsc::channel::<String>(16);

        // Clone Arcs for removal handler before they move into discovery handler
        let remove_agents = agents.clone();
        let remove_connections = agent_connections.clone();

        // Blocking thread: receive mDNS events, forward resolved/removed services
        tokio::task::spawn_blocking(move || {
            loop {
                if let Ok(event) = receiver.recv_timeout(Duration::from_secs(5)) {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            println!("AG-UI: mDNS ServiceResolved: {}", info.get_fullname());
                            if info_tx.blocking_send(info).is_err() {
                                break;
                            }
                        }
                        ServiceEvent::ServiceFound(_, fullname) => {
                            println!("AG-UI: mDNS ServiceFound: {fullname}");
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            println!("AG-UI: mDNS ServiceRemoved: {fullname}");
                            let _ = remove_tx.blocking_send(fullname);
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

        // Async task: handle discovered services
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

        // Async task: remove agents when mDNS service disappears
        tokio::spawn(async move {
            while let Some(fullname) = remove_rx.recv().await {
                handle_service_removed(&fullname, &remove_agents, &remove_connections).await;
            }
        });

        // Keep the daemon alive (it's dropped when this future completes)
        loop {
            tokio::time::sleep(Duration::from_hours(1)).await;
        }
    }

    /// Remove an agent when its mDNS service disappears.
    async fn handle_service_removed(
        fullname: &str,
        agents: &Arc<RwLock<Vec<serde_json::Value>>>,
        agent_connections: &Arc<
            RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>,
        >,
    ) {
        // Extract agent_id: fullname is like "commander._a2a._tcp.local."
        let agent_id = fullname
            .split("._a2a._tcp")
            .next()
            .unwrap_or(fullname)
            .to_string();

        let mut agents_list = agents.write().await;
        let before = agents_list.len();
        agents_list.retain(|a| {
            a.get("id")
                .and_then(|v| v.as_str())
                .is_none_or(|id| id != agent_id)
        });
        let removed = before - agents_list.len();

        if removed > 0 {
            println!("AG-UI: Removed agent from list: {agent_id}");
            let mut connections = agent_connections.write().await;
            connections.remove(&agent_id);
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
                                for ci in conns.values() {
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

    // Runs on macOS only, the one platform this was verified on. Its loopback
    // interface carries multicast; on Linux `lo` usually lacks the MULTICAST
    // flag, and whether mDNS works over it there has not been established.
    #[cfg(all(test, target_os = "macos"))]
    mod tests {
        use super::*;

        /// Announces an agent bound to 127.0.0.1 on the loopback interfaces and
        /// on no other, so the record can only reach a browser that listens
        /// there and nothing the test publishes leaves the machine.
        fn announce_on_loopback(agent_id: &str, port: u16) -> mdns_sd::ServiceDaemon {
            use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
            use std::net::{IpAddr, Ipv4Addr};

            let daemon = ServiceDaemon::new().expect("mDNS daemon");
            daemon
                .disable_interface(IfKind::All)
                .expect("leave every interface");
            daemon
                .enable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
                .expect("join the loopback interfaces");

            let mut properties = std::collections::HashMap::new();
            properties.insert("agent_id".to_string(), agent_id.to_string());
            properties.insert("purpose".to_string(), "Plans the work".to_string());
            let service = ServiceInfo::new(
                "_a2a._tcp.local.",
                agent_id,
                &format!("{agent_id}.local."),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                port,
                properties,
            )
            .expect("service info");
            daemon.register(service).expect("register the agent");
            daemon
        }

        /// Regression: the AG-UI browsed with a daemon that skips loopback
        /// interfaces, so an agent on the same machine, listening on the
        /// default address, never appeared in its agent list.
        #[test]
        fn the_browsing_daemon_finds_an_agent_on_loopback() {
            let agent_id = format!("loopback-agui-{}", std::process::id());
            let announcer = announce_on_loopback(&agent_id, 48_434);

            let browser = browsing_daemon().expect("browsing daemon");
            let receiver = browser.browse("_a2a._tcp.local.").expect("browse");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut found = None;
            while found.is_none() && std::time::Instant::now() < deadline {
                if let Ok(ServiceEvent::ServiceResolved(info)) =
                    receiver.recv_timeout(Duration::from_millis(100))
                    && info.get_property_val_str("agent_id") == Some(agent_id.as_str())
                {
                    found = Some(info);
                }
            }
            browser.shutdown().ok();
            announcer.shutdown().ok();

            let info = found.unwrap_or_else(|| panic!("{agent_id} was not resolved"));
            assert_eq!(info.get_port(), 48_434);
            assert!(
                info.get_addresses()
                    .iter()
                    .all(std::net::IpAddr::is_loopback),
                "{:?}",
                info.get_addresses()
            );
        }
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


}
