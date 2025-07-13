use crate::error::{A2aError, Result};
use crate::transport::A2aEndpoint;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

#[cfg(feature = "mdns")]
use crate::mdns::MdnsServiceInfo;

#[cfg(not(feature = "mdns"))]
pub struct MdnsServiceInfo;

#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub method: DiscoveryMethod,
    pub timeout_ms: u64,
    pub cache_ttl_seconds: u64,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            method: DiscoveryMethod::Static,
            timeout_ms: 5000,
            cache_ttl_seconds: 300,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DiscoveryMethod {
    Static,
    Dns,
    Environment,
    #[cfg(feature = "mdns")]
    Mdns,
}

pub struct DiscoveryService {
    config: DiscoveryConfig,
    static_endpoints: Arc<RwLock<HashMap<String, A2aEndpoint>>>,
    discovered_endpoints: Arc<RwLock<HashMap<String, A2aEndpoint>>>,
}

impl DiscoveryService {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self {
            config,
            static_endpoints: Arc::new(RwLock::new(HashMap::new())),
            discovered_endpoints: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers a static endpoint for an agent.
    ///
    /// # Panics
    ///
    /// Panics if the RwLock is poisoned.
    pub fn register_static_endpoint(&self, endpoint: A2aEndpoint) {
        let mut endpoints = self.static_endpoints.write().unwrap();
        info!(
            "Registering static endpoint: {} at {}",
            endpoint.agent_id, endpoint.url
        );
        endpoints.insert(endpoint.agent_id.clone(), endpoint);
    }

    pub fn discover_agent(&self, agent_id: &str) -> Result<A2aEndpoint> {
        if let Some(endpoint) = self.check_static_endpoints(agent_id) {
            return Ok(endpoint);
        }

        if let Some(endpoint) = self.check_discovered_cache(agent_id) {
            return Ok(endpoint);
        }

        match self.config.method {
            DiscoveryMethod::Static => Err(A2aError::AgentNotFound(format!(
                "Agent {agent_id} not found in static registry"
            ))),
            DiscoveryMethod::Environment => self.discover_from_environment(agent_id),
            DiscoveryMethod::Dns => self.discover_via_dns(agent_id),
            #[cfg(feature = "mdns")]
            DiscoveryMethod::Mdns => Err(A2aError::ServiceDiscovery(
                "mDNS discovery should be initiated through AG-UI gateway".to_string(),
            )),
        }
    }

    /// Discovers all available agents.
    ///
    /// # Panics
    ///
    /// Panics if the RwLock is poisoned.
    pub fn discover_all(&self) -> Result<Vec<A2aEndpoint>> {
        let mut all_endpoints = Vec::new();

        {
            let static_endpoints = self.static_endpoints.read().unwrap();
            all_endpoints.extend(static_endpoints.values().cloned());
        }

        match self.config.method {
            DiscoveryMethod::Static => {}
            DiscoveryMethod::Environment => {
                if let Ok(endpoints) = self.discover_all_from_environment() {
                    all_endpoints.extend(endpoints);
                }
            }
            DiscoveryMethod::Dns => {
                warn!("DNS discovery for all agents not implemented");
            }
            #[cfg(feature = "mdns")]
            DiscoveryMethod::Mdns => {
                warn!("mDNS discovery for all agents should be initiated through AG-UI gateway");
            }
        }

        Ok(all_endpoints)
    }

    fn check_static_endpoints(&self, agent_id: &str) -> Option<A2aEndpoint> {
        let endpoints = self.static_endpoints.read().unwrap();
        endpoints.get(agent_id).cloned()
    }

    fn check_discovered_cache(&self, agent_id: &str) -> Option<A2aEndpoint> {
        let endpoints = self.discovered_endpoints.read().unwrap();
        endpoints.get(agent_id).cloned()
    }

    fn discover_from_environment(&self, agent_id: &str) -> Result<A2aEndpoint> {
        let env_var = format!(
            "A2A_AGENT_{}_URL",
            agent_id.to_uppercase().replace('-', "_")
        );
        debug!("Checking environment variable: {}", env_var);

        match std::env::var(&env_var) {
            Ok(url) => {
                let endpoint = A2aEndpoint {
                    url,
                    agent_id: agent_id.to_string(),
                    public_key: None,
                };

                {
                    let mut discovered = self.discovered_endpoints.write().unwrap();
                    discovered.insert(agent_id.to_string(), endpoint.clone());
                }

                info!("Discovered agent {} from environment", agent_id);
                Ok(endpoint)
            }
            Err(_) => Err(A2aError::AgentNotFound(format!(
                "Environment variable {env_var} not found"
            ))),
        }
    }

    fn discover_all_from_environment(&self) -> Result<Vec<A2aEndpoint>> {
        let mut endpoints = Vec::new();

        for (key, value) in std::env::vars() {
            if key.starts_with("A2A_AGENT_") && key.ends_with("_URL") {
                let agent_id = key
                    .trim_start_matches("A2A_AGENT_")
                    .trim_end_matches("_URL")
                    .to_lowercase()
                    .replace('_', "-");

                let endpoint = A2aEndpoint {
                    url: value,
                    agent_id: agent_id.clone(),
                    public_key: None,
                };

                endpoints.push(endpoint.clone());

                {
                    let mut discovered = self.discovered_endpoints.write().unwrap();
                    discovered.insert(agent_id, endpoint);
                }
            }
        }

        Ok(endpoints)
    }

    fn discover_via_dns(&self, agent_id: &str) -> Result<A2aEndpoint> {
        warn!(
            "DNS SRV discovery not yet implemented for agent: {}",
            agent_id
        );
        Err(A2aError::ServiceDiscovery(
            "DNS discovery not implemented".to_string(),
        ))
    }

    /// Clears the discovered endpoints cache.
    ///
    /// # Panics
    ///
    /// Panics if the RwLock is poisoned.
    pub fn clear_cache(&self) {
        self.discovered_endpoints.write().unwrap().clear();
        info!("Cleared discovery cache");
    }

    /// Start mDNS discovery for A2A agents
    ///
    /// This method starts discovering A2A agents on the local network using mDNS.
    /// Note: This is a placeholder implementation. The actual mDNS discovery logic
    /// is implemented in the arkavo-agui crate's gateway module.
    #[cfg(feature = "mdns")]
    pub async fn start_mdns_discovery(&self) -> Result<()> {
        info!("Starting mDNS discovery for A2A agents");

        // The actual mDNS discovery implementation is in arkavo-agui/src/gateway.rs
        // This method is provided for API compatibility with the example
        warn!("mDNS discovery should be started through the AG-UI gateway");

        Err(A2aError::ServiceDiscovery(
            "mDNS discovery is implemented in the AG-UI gateway, not the protocol layer"
                .to_string(),
        ))
    }

    #[cfg(not(feature = "mdns"))]
    pub async fn start_mdns_discovery(&self) -> Result<()> {
        Err(A2aError::ServiceDiscovery(
            "mDNS feature not compiled in".to_string(),
        ))
    }

    /// Register an mDNS service for an A2A agent
    ///
    /// This method registers an A2A agent as an mDNS service on the local network.
    /// Note: This is a placeholder implementation. The actual mDNS registration logic
    /// is implemented in the arkavo-cli crate's agent module.
    #[cfg(feature = "mdns")]
    pub async fn register_mdns_service(&self, _service_info: MdnsServiceInfo) -> Result<()> {
        info!("Registering mDNS service for A2A agent");

        // The actual mDNS registration implementation is in arkavo-cli/src/commands/agent.rs
        // This method is provided for API compatibility with the example
        warn!("mDNS service registration should be done through the agent command");

        Err(A2aError::ServiceDiscovery(
            "mDNS service registration is implemented in the agent command, not the protocol layer"
                .to_string(),
        ))
    }

    #[cfg(not(feature = "mdns"))]
    pub async fn register_mdns_service(&self, _service_info: MdnsServiceInfo) -> Result<()> {
        Err(A2aError::ServiceDiscovery(
            "mDNS feature not compiled in".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_endpoint_registration() {
        let service = DiscoveryService::new(DiscoveryConfig::default());

        let endpoint = A2aEndpoint {
            url: "http://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        service.register_static_endpoint(endpoint.clone());

        let found = service.check_static_endpoints("test-agent");
        assert!(found.is_some());
        assert_eq!(found.unwrap().url, "http://localhost:8080");
    }

    #[test]
    fn test_static_discovery() {
        let service = DiscoveryService::new(DiscoveryConfig {
            method: DiscoveryMethod::Static,
            ..Default::default()
        });

        let endpoint = A2aEndpoint {
            url: "http://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        service.register_static_endpoint(endpoint);

        let result = service.discover_agent("test-agent");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().url, "http://localhost:8080");
    }

    #[test]
    fn test_agent_not_found() {
        let service = DiscoveryService::new(DiscoveryConfig::default());

        let result = service.discover_agent("unknown-agent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_discover_all() {
        let service = DiscoveryService::new(DiscoveryConfig::default());

        service.register_static_endpoint(A2aEndpoint {
            url: "http://localhost:8080".to_string(),
            agent_id: "agent-1".to_string(),
            public_key: None,
        });

        service.register_static_endpoint(A2aEndpoint {
            url: "http://localhost:8081".to_string(),
            agent_id: "agent-2".to_string(),
            public_key: None,
        });

        let all = service.discover_all().unwrap();
        assert_eq!(all.len(), 2);
    }
}
