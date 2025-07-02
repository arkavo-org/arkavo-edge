use crate::discovery::{DiscoveryConfig, DiscoveryMethod};
use crate::rate_limit::RateLimitConfig;
use crate::security::SecurityConfig;
use crate::transport::TransportConfig;
use anyhow::Result;
use tracing::info;

#[derive(Debug, Clone)]
pub struct A2aConfig {
    pub agent_id: String,
    pub transport: TransportConfig,
    pub security: SecurityConfig,
    pub discovery: DiscoveryConfig,
    pub server: ServerConfig,
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            agent_id: generate_agent_id(),
            transport: TransportConfig::default(),
            security: SecurityConfig::default(),
            discovery: DiscoveryConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub port: u16,
    pub max_connections: usize,
    pub idle_timeout_seconds: u64,
    pub rate_limit: RateLimitConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: "0.0.0.0".to_string(),
            port: 8765,
            max_connections: 100,
            idle_timeout_seconds: 300,
            rate_limit: RateLimitConfig::default(),
        }
    }
}

impl A2aConfig {
    pub fn builder() -> A2aConfigBuilder {
        A2aConfigBuilder::default()
    }

    pub fn validate(&self) -> Result<()> {
        if self.agent_id.is_empty() {
            return Err(anyhow::anyhow!("Agent ID cannot be empty"));
        }

        if self.server.enabled && self.server.port == 0 {
            return Err(anyhow::anyhow!(
                "Server port cannot be 0 when server is enabled"
            ));
        }

        self.security.validate()?;

        Ok(())
    }

    pub fn from_environment() -> Self {
        let mut config = Self::default();

        if let Ok(agent_id) = std::env::var("A2A_AGENT_ID") {
            config.agent_id = agent_id;
        }

        if let Ok(timeout) = std::env::var("A2A_TIMEOUT_MS") {
            if let Ok(timeout_ms) = timeout.parse::<u64>() {
                config.transport.timeout_ms = timeout_ms;
            }
        }

        if let Ok(port) = std::env::var("A2A_SERVER_PORT") {
            if let Ok(port_num) = port.parse::<u16>() {
                config.server.port = port_num;
                config.server.enabled = true;
            }
        }

        if let Ok(discovery_method) = std::env::var("A2A_DISCOVERY_METHOD") {
            config.discovery.method = match discovery_method.to_lowercase().as_str() {
                "static" => DiscoveryMethod::Static,
                "dns" => DiscoveryMethod::Dns,
                _ => DiscoveryMethod::Static,
            };
        }

        info!(
            "Loaded A2A configuration from environment for agent: {}",
            config.agent_id
        );
        config
    }
}

#[derive(Default)]
pub struct A2aConfigBuilder {
    config: A2aConfig,
}

impl A2aConfigBuilder {
    pub fn agent_id(mut self, id: impl Into<String>) -> Self {
        self.config.agent_id = id.into();
        self
    }

    pub fn transport_timeout(mut self, timeout_ms: u64) -> Self {
        self.config.transport.timeout_ms = timeout_ms;
        self
    }

    pub fn retry_policy(mut self, max_retries: u32, delay_ms: u64) -> Self {
        self.config.transport.max_retries = max_retries;
        self.config.transport.retry_delay_ms = delay_ms;
        self
    }

    pub fn tls_required(mut self, required: bool) -> Self {
        self.config.transport.tls_config.require_tls = required;
        self.config.security.tls.enabled = required;
        self
    }

    pub fn discovery_method(mut self, method: DiscoveryMethod) -> Self {
        self.config.discovery.method = method;
        self
    }

    pub fn server_enabled(mut self, enabled: bool) -> Self {
        self.config.server.enabled = enabled;
        self
    }

    pub fn server_bind(mut self, address: impl Into<String>, port: u16) -> Self {
        self.config.server.bind_address = address.into();
        self.config.server.port = port;
        self
    }

    pub fn max_connections(mut self, max: usize) -> Self {
        self.config.server.max_connections = max;
        self
    }

    pub fn build(self) -> Result<A2aConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

fn generate_agent_id() -> String {
    format!(
        "agent-{}",
        uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
    )
}

pub struct ConfigManager {
    config: A2aConfig,
}

impl ConfigManager {
    pub fn new(config: A2aConfig) -> Self {
        Self { config }
    }

    pub fn from_environment() -> Self {
        Self::new(A2aConfig::from_environment())
    }

    pub fn get(&self) -> &A2aConfig {
        &self.config
    }

    pub fn update<F>(&mut self, updater: F) -> Result<()>
    where
        F: FnOnce(&mut A2aConfig),
    {
        updater(&mut self.config);
        self.config.validate()?;
        Ok(())
    }

    pub fn reload_from_environment(&mut self) {
        self.config = A2aConfig::from_environment();
        info!("Reloaded configuration from environment");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = A2aConfig::default();
        assert!(!config.agent_id.is_empty());
        assert!(!config.server.enabled);
        assert_eq!(config.server.port, 8765);
    }

    #[test]
    fn test_config_builder() {
        let config = A2aConfig::builder()
            .agent_id("test-agent")
            .transport_timeout(10000)
            .retry_policy(5, 2000)
            .server_enabled(true)
            .server_bind("127.0.0.1", 9000)
            .build()
            .unwrap();

        assert_eq!(config.agent_id, "test-agent");
        assert_eq!(config.transport.timeout_ms, 10000);
        assert_eq!(config.transport.max_retries, 5);
        assert_eq!(config.transport.retry_delay_ms, 2000);
        assert!(config.server.enabled);
        assert_eq!(config.server.bind_address, "127.0.0.1");
        assert_eq!(config.server.port, 9000);
    }

    #[test]
    fn test_validation_empty_agent_id() {
        let mut config = A2aConfig::default();
        config.agent_id = String::new();

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Agent ID cannot be empty")
        );
    }

    #[test]
    fn test_config_manager() {
        let mut manager = ConfigManager::new(A2aConfig::default());

        let original_id = manager.get().agent_id.clone();

        manager
            .update(|config| {
                config.server.enabled = true;
                config.server.port = 10000;
            })
            .unwrap();

        assert_eq!(manager.get().agent_id, original_id);
        assert!(manager.get().server.enabled);
        assert_eq!(manager.get().server.port, 10000);
    }
}
