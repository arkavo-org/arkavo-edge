use crate::error::{A2aError, Result};
use crate::transport::A2aEndpoint;
#[cfg(feature = "mdns")]
use crate::mdns::{MdnsManager, MdnsServiceInfo};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

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
    Mdns,
    Dns,
    Environment,
}

pub struct DiscoveryService {
    config: DiscoveryConfig,
    static_endpoints: Arc<RwLock<HashMap<String, A2aEndpoint>>>,
    discovered_endpoints: Arc<RwLock<HashMap<String, A2aEndpoint>>>,
    #[cfg(feature = "mdns")]
    mdns_manager: Arc<MdnsManager>,
}

impl DiscoveryService {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self {
            config,
            static_endpoints: Arc::new(RwLock::new(HashMap::new())),
            discovered_endpoints: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "mdns")]
            mdns_manager: Arc::new(MdnsManager::new()),
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
            DiscoveryMethod::Mdns => self.discover_via_mdns(agent_id),
            DiscoveryMethod::Dns => self.discover_via_dns(agent_id),
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
            DiscoveryMethod::Mdns => {
                if let Ok(endpoints) = self.discover_all_via_mdns() {
                    all_endpoints.extend(endpoints);
                }
            }
            DiscoveryMethod::Dns => {
                warn!("DNS discovery for all agents not implemented");
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

    fn discover_via_mdns(&self, agent_id: &str) -> Result<A2aEndpoint> {
        #[cfg(feature = "mdns")]
        {
            // Note: This is a synchronous check of already discovered agents
            // Actual discovery happens asynchronously in the background
            let endpoint_opt = {
                let endpoints = self.discovered_endpoints.read().unwrap();
                endpoints.get(agent_id).cloned()
            };
            
            if let Some(endpoint) = endpoint_opt {
                info!("Found agent {} via mDNS at {}", agent_id, endpoint.url);
                return Ok(endpoint);
            }
            
            warn!("Agent {} not found via mDNS", agent_id);
            Err(A2aError::AgentNotFound(format!(
                "Agent {agent_id} not found via mDNS discovery"
            )))
        }
        
        #[cfg(not(feature = "mdns"))]
        {
            warn!("mDNS discovery not available (compile with 'mdns' feature)");
            Err(A2aError::ServiceDiscovery(
                "mDNS discovery not available".to_string(),
            ))
        }
    }

    fn discover_all_via_mdns(&self) -> Result<Vec<A2aEndpoint>> {
        #[cfg(feature = "mdns")]
        {
            // Return all mDNS discovered endpoints from cache
            let endpoints: Vec<A2aEndpoint> = self.discovered_endpoints.read().unwrap().values().cloned().collect();
            info!("Found {} agents via mDNS", endpoints.len());
            Ok(endpoints)
        }
        
        #[cfg(not(feature = "mdns"))]
        {
            warn!("mDNS discovery not available (compile with 'mdns' feature)");
            Ok(Vec::new())
        }
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

    /// Start mDNS discovery
    /// 
    /// # Panics
    /// 
    /// Panics if the RwLock is poisoned
    #[cfg(feature = "mdns")]
    pub async fn start_mdns_discovery(&self) -> Result<()> {
        self.mdns_manager.start_discovery().await?;
        
        // Also share discovered endpoints with the main discovery cache
        let mdns_manager = self.mdns_manager.clone();
        let discovered_endpoints = self.discovered_endpoints.clone();
        
        tokio::spawn(async move {
            loop {
                // Sync mDNS discoveries to main cache
                let mdns_endpoints = mdns_manager.get_discovered_services().await;
                {
                    let mut cache = discovered_endpoints.write().unwrap();
                    for endpoint in mdns_endpoints {
                        cache.insert(endpoint.agent_id.clone(), endpoint);
                    }
                }  // cache lock is dropped here
                
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });
        
        info!("Started mDNS discovery");
        Ok(())
    }

    /// Register this agent with mDNS
    #[cfg(feature = "mdns")]
    pub async fn register_mdns_service(&self, info: MdnsServiceInfo) -> Result<()> {
        self.mdns_manager.register_service(info).await?;
        info!("Registered mDNS service");
        Ok(())
    }

    /// Shutdown mDNS operations
    #[cfg(feature = "mdns")]
    pub async fn shutdown_mdns(&self) -> Result<()> {
        self.mdns_manager.shutdown().await?;
        info!("Shutdown mDNS operations");
        Ok(())
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
