//! mDNS implementation using pure Rust mdns-sd crate

#[cfg(feature = "mdns")]
pub mod mdns {
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{RwLock, mpsc};
    use tracing::{debug, error, info, warn};

    /// Discovers A2A agents using mDNS
    pub async fn discover_agents(
        agents: Arc<RwLock<Vec<serde_json::Value>>>,
        agent_connections: Arc<
            RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>,
        >,
        telemetry_tx: mpsc::Sender<crate::agent_connection::TelemetryEvent>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting mDNS discovery service with mdns-sd...");

        // Create the mDNS daemon
        let mdns = ServiceDaemon::new()?;

        // Browse for _a2a._tcp services
        let service_type = "_a2a._tcp.local.";
        let receiver = mdns.browse(service_type)?;

        // Spawn a blocking task for mDNS discovery to avoid blocking the async runtime
        // The mdns-sd receiver uses blocking I/O, so we run it on a dedicated thread
        let agents_clone = agents.clone();
        let connections_clone = agent_connections.clone();
        let telemetry_clone = telemetry_tx.clone();
        let rt = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            loop {
                match receiver.recv_timeout(Duration::from_millis(500)) {
                    Ok(event) => match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let agents = agents_clone.clone();
                            let connections = connections_clone.clone();
                            let telemetry = telemetry_clone.clone();
                            rt.spawn(async move {
                                handle_service_discovered(info, agents, connections, telemetry)
                                    .await;
                            });
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            info!("Service removed: {}", fullname);
                        }
                        _ => {}
                    },
                    Err(_) => {
                        // Timeout - continue loop
                    }
                }
            }
        });

        // Keep the daemon running
        tokio::time::sleep(Duration::from_secs(3600)).await;

        Ok(())
    }

    async fn handle_service_discovered(
        info: ServiceInfo,
        agents: Arc<RwLock<Vec<serde_json::Value>>>,
        agent_connections: Arc<
            RwLock<HashMap<String, Arc<crate::agent_connection::AgentConnection>>>,
        >,
        telemetry_tx: mpsc::Sender<crate::agent_connection::TelemetryEvent>,
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

        info!("Discovered service: {} at {}:{}", service_name, host, port);

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
            .unwrap_or_else(|| "Unknown".to_string());

        // Extract agent_id from properties if available
        if let Some(id_prop) = properties.get("agent_id") {
            agent_id = id_prop.val_str().to_string();
        }

        // Use IP from properties if host is 0.0.0.0
        let final_host = if host == "0.0.0.0" {
            if let Some(ip_prop) = properties.get("ip") {
                ip_prop.val_str().to_string()
            } else {
                warn!("Service advertised 0.0.0.0 with no IP in TXT records, using 127.0.0.1");
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
            info!("Adding new agent to list: {}", agent_id);

            // Auto-connect to discovered agent
            let agent_id_clone = agent_id.clone();
            let endpoint = format!("{}:{}", final_host, port);
            let telemetry_tx_clone = telemetry_tx.clone();
            let agent_connections_clone = agent_connections.clone();

            debug!(
                agent_id = %agent_id_clone,
                endpoint = %endpoint,
                "Spawning auto-connect task"
            );

            tokio::spawn(async move {
                debug!(
                    agent_id = %agent_id_clone,
                    endpoint = %endpoint,
                    "Auto-connect task started"
                );

                info!(
                    "Auto-connecting to discovered agent: {} at {}",
                    agent_id_clone, endpoint
                );

                // Create connection
                let connection = Arc::new(crate::agent_connection::AgentConnection::new(
                    agent_id_clone.clone(),
                    endpoint.clone(),
                    telemetry_tx_clone,
                ));

                debug!(agent_id = %agent_id_clone, "Calling connection.connect()");

                // Connect to the agent
                match connection.connect().await {
                    Err(e) => {
                        error!("Failed to auto-connect to agent {}: {}", agent_id_clone, e);
                    }
                    Ok(()) => {
                        info!("Successfully auto-connected to agent: {}", agent_id_clone);

                        // Store connection
                        let mut connections = agent_connections_clone.write().await;
                        connections.insert(agent_id_clone.clone(), connection);
                        debug!(
                            agent_id = %agent_id_clone,
                            total_connections = connections.len(),
                            "Stored agent connection"
                        );
                    }
                }
            });

            agents_list.push(agent_info);
        }
    }

    /// Registers an A2A agent as an mDNS service
    pub fn register_service(
        agent_id: &str,
        port: u16,
        purpose: &str,
        model: &str,
    ) -> Result<ServiceDaemon, Box<dyn std::error::Error>> {
        info!("Registering mDNS service for agent: {}", agent_id);

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

        info!("mDNS service registered successfully");
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
