use crate::error::{A2aError, Result};
use crate::transport::A2aEndpoint;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
#[cfg(not(feature = "mdns"))]
use tracing::warn;
use tracing::{debug, info};

#[cfg(feature = "mdns")]
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

/// mDNS service information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdnsServiceInfo {
    pub agent_id: String,
    pub http_port: u16,
    pub ws_port: Option<u16>,
    pub version: String,
    pub capabilities: Vec<String>,
}

/// mDNS service manager for A2A protocol
pub struct MdnsManager {
    service_type: String,
    #[cfg(feature = "mdns")]
    daemon: Arc<Mutex<Option<ServiceDaemon>>>,
    discovered_services: Arc<RwLock<HashMap<String, A2aEndpoint>>>,
}

impl MdnsManager {
    /// Create a new mDNS manager
    pub fn new() -> Self {
        Self {
            service_type: "_a2a._tcp.local.".to_string(),
            #[cfg(feature = "mdns")]
            daemon: Arc::new(Mutex::new(None)),
            discovered_services: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Initialize the mDNS daemon
    #[cfg(feature = "mdns")]
    async fn ensure_daemon(&self) -> Result<()> {
        let mut daemon_guard = self.daemon.lock().await;
        if daemon_guard.is_none() {
            println!("mDNS: Creating new ServiceDaemon...");
            let daemon = ServiceDaemon::new().map_err(|e| {
                eprintln!("mDNS: Failed to create ServiceDaemon: {e}");
                A2aError::ServiceDiscovery(format!("Failed to create mDNS daemon: {e}"))
            })?;
            println!("mDNS: ServiceDaemon created successfully");
            *daemon_guard = Some(daemon);
            drop(daemon_guard);
        } else {
            println!("mDNS: ServiceDaemon already exists");
        }
        Ok(())
    }

    /// Register this agent as an mDNS service
    #[cfg(feature = "mdns")]
    pub async fn register_service(&self, info: MdnsServiceInfo) -> Result<()> {
        self.ensure_daemon().await?;

        let service_name = format!("arkavo-agent-{}", info.agent_id);
        // Use the local machine's hostname for mDNS
        let mut hostname = gethostname::gethostname().to_string_lossy().into_owned();

        // Fix hostname to be a proper FQDN
        // Strip any trailing dots first
        while hostname.ends_with('.') {
            hostname.pop();
        }

        // Add .local if not already present
        if !hostname.ends_with(".local") {
            hostname.push_str(".local");
        }

        // Add final dot for FQDN
        hostname.push('.');

        // Create properties with service info
        let mut properties = HashMap::new();
        properties.insert("agent_id".to_string(), info.agent_id.clone());
        properties.insert("version".to_string(), info.version.clone());
        properties.insert("http_port".to_string(), info.http_port.to_string());

        if let Some(ws_port) = info.ws_port {
            properties.insert("ws_port".to_string(), ws_port.to_string());
        }

        // Add capabilities
        for (i, cap) in info.capabilities.iter().enumerate() {
            properties.insert(format!("cap_{i}"), cap.clone());
        }

        println!("mDNS: Creating ServiceInfo with:");
        println!("  - service_type: {}", self.service_type);
        println!("  - service_name: {service_name}");
        println!("  - hostname: {hostname}");
        println!("  - port: {}", info.http_port);

        let service_info = ServiceInfo::new(
            &self.service_type,
            &service_name,
            &hostname,
            (), // Will auto-detect IP addresses
            info.http_port,
            Some(properties),
        )
        .map_err(|e| A2aError::ServiceDiscovery(format!("Failed to create service info: {e}")))?;

        if let Some(daemon) = self.daemon.lock().await.as_ref() {
            println!("mDNS: About to call daemon.register()...");
            daemon.register(service_info).map_err(|e| {
                eprintln!("mDNS: daemon.register() failed: {e}");
                A2aError::ServiceDiscovery(format!("Failed to register service: {e}"))
            })?;
            println!("mDNS: daemon.register() succeeded!");

            println!(
                "mDNS: Registered service: {} on port {} with type {}",
                service_name, info.http_port, self.service_type
            );
            info!(
                "Registered mDNS service: {} on port {}",
                service_name, info.http_port
            );
        } else {
            eprintln!("mDNS: No daemon available!");
        }

        Ok(())
    }

    /// Register a service (stub for non-mdns builds)
    #[cfg(not(feature = "mdns"))]
    pub async fn register_service(&self, _info: MdnsServiceInfo) -> Result<()> {
        warn!("mDNS support not enabled. Compile with 'mdns' feature.");
        Ok(())
    }

    /// Start browsing for mDNS services
    #[cfg(feature = "mdns")]
    pub async fn start_discovery(&self) -> Result<()> {
        self.ensure_daemon().await?;

        let discovered = self.discovered_services.clone();
        let service_type = self.service_type.clone();

        if let Some(daemon) = self.daemon.lock().await.as_ref() {
            println!("mDNS: Creating browser for service type: {service_type}");
            let receiver = daemon
                .browse(&service_type)
                .map_err(|e| A2aError::ServiceDiscovery(format!("Failed to start browser: {e}")))?;
            println!("mDNS: Browser created successfully");

            // Spawn task to handle discovered services
            tokio::spawn(async move {
                while let Ok(event) = receiver.recv() {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let fullname = info.get_fullname();
                            println!("mDNS: ServiceResolved event for: {fullname}");
                            debug!("Discovered service: {}", fullname);

                            // Extract agent info from properties
                            let properties = info.get_properties();
                            if let (Some(agent_id_prop), Some(http_port_prop)) =
                                (properties.get("agent_id"), properties.get("http_port"))
                            {
                                let agent_id = agent_id_prop.val_str();
                                if let Ok(http_port) = http_port_prop.val_str().parse::<u16>() {
                                    // Get addresses
                                    let addresses = info.get_addresses();
                                    if let Some(addr) = addresses.iter().next() {
                                        let endpoint = A2aEndpoint {
                                            agent_id: agent_id.to_string(),
                                            url: format!("http://{addr}:{http_port}"),
                                            public_key: None,
                                        };

                                        discovered
                                            .write()
                                            .await
                                            .insert(agent_id.to_string(), endpoint);
                                        info!("Added mDNS discovered agent: {}", agent_id);

                                        // TODO: Add metrics reporting when metrics collector is available
                                        // metrics.record_mdns_discovery();
                                    }
                                }
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            debug!("Service removed: {}", fullname);
                            // Extract agent_id from fullname if possible
                            if let Some(agent_id) = fullname
                                .split('.')
                                .next()
                                .and_then(|s| s.strip_prefix("arkavo-agent-"))
                            {
                                discovered.write().await.remove(agent_id);
                                info!("Removed mDNS agent: {}", agent_id);
                            }
                        }
                        _ => {}
                    }
                }
            });
        }

        println!("mDNS: Started discovery for {} services", self.service_type);
        info!("Started mDNS discovery for {} services", self.service_type);
        Ok(())
    }

    /// Start discovery (stub for non-mdns builds)
    #[cfg(not(feature = "mdns"))]
    pub async fn start_discovery(&self) -> Result<()> {
        warn!("mDNS support not enabled. Compile with 'mdns' feature.");
        Ok(())
    }

    /// Get all discovered services
    pub async fn get_discovered_services(&self) -> Vec<A2aEndpoint> {
        self.discovered_services
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }

    /// Find a specific agent by ID
    pub async fn find_agent(&self, agent_id: &str) -> Option<A2aEndpoint> {
        self.discovered_services.read().await.get(agent_id).cloned()
    }

    /// Stop mDNS operations
    #[cfg(feature = "mdns")]
    pub async fn shutdown(&self) -> Result<()> {
        let daemon_opt = self.daemon.lock().await.take();
        if let Some(daemon) = daemon_opt {
            daemon.shutdown().map_err(|e| {
                A2aError::ServiceDiscovery(format!("Failed to shutdown daemon: {e}"))
            })?;
            info!("Shutdown mDNS daemon");
        }
        Ok(())
    }

    /// Shutdown (stub for non-mdns builds)
    #[cfg(not(feature = "mdns"))]
    pub async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

impl Default for MdnsManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mdns_service_info() {
        let info = MdnsServiceInfo {
            agent_id: "test-agent".to_string(),
            http_port: 8080,
            ws_port: Some(8081),
            version: "0.1.0".to_string(),
            capabilities: vec!["promise_request".to_string()],
        };

        assert_eq!(info.agent_id, "test-agent");
        assert_eq!(info.http_port, 8080);
        assert_eq!(info.ws_port, Some(8081));
    }

    #[tokio::test]
    async fn test_mdns_manager_creation() {
        let manager = MdnsManager::new();
        assert!(manager.get_discovered_services().await.is_empty());
    }
}
