use crate::discovery::{DiscoveryConfig, DiscoveryMethod};
use crate::rate_limit::RateLimitConfig;
use crate::security::SecurityConfig;
use crate::transport::TransportConfig;
use anyhow::Result;
use arkavo_observability::config::SecureConfigProvider;
use tracing::{info, instrument, warn};

#[derive(Debug, Clone)]
pub struct BufferConfig {
    pub chat_delta_buffer_size: usize,
    pub metrics_broadcast_buffer_size: usize,
    pub telemetry_channel_buffer_size: usize,
    pub agui_broadcast_buffer_size: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            chat_delta_buffer_size: 32,
            metrics_broadcast_buffer_size: 100,
            telemetry_channel_buffer_size: 1000,
            agui_broadcast_buffer_size: 32,
        }
    }
}

impl BufferConfig {
    /// Validate buffer configuration and warn on invalid values
    pub fn validate(&self) -> Result<()> {
        const MIN_BUFFER_SIZE: usize = 1;
        const MAX_BUFFER_SIZE: usize = 4096;
        const WARN_MIN_SIZE: usize = 0;
        const WARN_MAX_SIZE: usize = 10_000;

        // Helper to check and warn about buffer sizes
        let check_buffer = |name: &str, size: usize| {
            if size == WARN_MIN_SIZE {
                warn!(
                    "{} is set to 0, which may cause blocking. Consider using a value between {} and {}",
                    name, MIN_BUFFER_SIZE, MAX_BUFFER_SIZE
                );
            } else if size > WARN_MAX_SIZE {
                warn!(
                    "{} is set to {}, which is very large and may consume excessive memory. Consider using a value between {} and {}",
                    name, size, MIN_BUFFER_SIZE, MAX_BUFFER_SIZE
                );
            } else if !(MIN_BUFFER_SIZE..=MAX_BUFFER_SIZE).contains(&size) {
                warn!(
                    "{} is set to {}, which is outside the recommended range of {} to {}",
                    name, size, MIN_BUFFER_SIZE, MAX_BUFFER_SIZE
                );
            }
        };

        check_buffer("chat_delta_buffer_size", self.chat_delta_buffer_size);
        check_buffer(
            "metrics_broadcast_buffer_size",
            self.metrics_broadcast_buffer_size,
        );
        check_buffer(
            "telemetry_channel_buffer_size",
            self.telemetry_channel_buffer_size,
        );
        check_buffer(
            "agui_broadcast_buffer_size",
            self.agui_broadcast_buffer_size,
        );

        // Don't fail on validation - just warn
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct A2aConfig {
    pub agent_id: String,
    pub transport: TransportConfig,
    pub security: SecurityConfig,
    pub discovery: DiscoveryConfig,
    pub server: ServerConfig,
    pub buffers: BufferConfig,
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            agent_id: generate_agent_id(),
            transport: TransportConfig::default(),
            security: SecurityConfig::default(),
            discovery: DiscoveryConfig::default(),
            server: ServerConfig::default(),
            buffers: BufferConfig::default(),
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
    pub task_store_path: Option<String>,
    pub metrics_enabled: bool,
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
            task_store_path: Some(".arkavo/arkavo_tasks.db".to_string()),
            metrics_enabled: true,
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
        self.buffers.validate()?;

        Ok(())
    }

    /// Load configuration from secure config provider instead of direct environment access
    #[instrument(skip(config_provider))]
    pub fn from_secure_config(config_provider: &SecureConfigProvider) -> Result<Self> {
        let mut config = Self::default();

        // Load agent ID with validation
        if let Some(agent_id) = config_provider.get_config("A2A_AGENT_ID")? {
            config.agent_id = agent_id;
        }

        // Load timeout with validation and parsing
        if let Some(timeout_str) = config_provider.get_config("A2A_TIMEOUT_MS")? {
            match timeout_str.parse::<u64>() {
                Ok(timeout_ms) => {
                    config.transport.timeout_ms = timeout_ms;
                    info!(timeout_ms = timeout_ms, "Transport timeout configured");
                }
                Err(e) => {
                    warn!(timeout_str = %timeout_str, error = %e, "Invalid timeout value, using default");
                }
            }
        }

        // Load server port with validation
        if let Some(port_str) = config_provider.get_config("A2A_SERVER_PORT")? {
            match port_str.parse::<u16>() {
                Ok(port_num) => {
                    config.server.port = port_num;
                    config.server.enabled = true;
                    info!(port = port_num, "Server port configured and enabled");
                }
                Err(e) => {
                    warn!(port_str = %port_str, error = %e, "Invalid port value, using default");
                }
            }
        }

        // Load discovery method with validation
        if let Some(discovery_method) = config_provider.get_config("A2A_DISCOVERY_METHOD")? {
            config.discovery.method = match discovery_method.to_lowercase().as_str() {
                "static" => DiscoveryMethod::Static,
                "dns" => DiscoveryMethod::Dns,
                _ => {
                    warn!(method = %discovery_method, "Unknown discovery method, using static");
                    DiscoveryMethod::Static
                }
            };
            info!(method = %discovery_method, "Discovery method configured");
        }

        // Load buffer sizes with validation
        if let Some(size_str) = config_provider.get_config("A2A_CHAT_DELTA_BUFFER_SIZE")?
            && let Ok(size) = size_str.parse::<usize>() {
                config.buffers.chat_delta_buffer_size = size;
                info!(size = size, "Chat delta buffer size configured");
            }

        if let Some(size_str) = config_provider.get_config("A2A_METRICS_BROADCAST_BUFFER_SIZE")?
            && let Ok(size) = size_str.parse::<usize>() {
                config.buffers.metrics_broadcast_buffer_size = size;
                info!(size = size, "Metrics broadcast buffer size configured");
            }

        if let Some(size_str) = config_provider.get_config("A2A_TELEMETRY_CHANNEL_BUFFER_SIZE")?
            && let Ok(size) = size_str.parse::<usize>() {
                config.buffers.telemetry_channel_buffer_size = size;
                info!(size = size, "Telemetry channel buffer size configured");
            }

        if let Some(size_str) = config_provider.get_config("A2A_AGUI_BROADCAST_BUFFER_SIZE")?
            && let Ok(size) = size_str.parse::<usize>() {
                config.buffers.agui_broadcast_buffer_size = size;
                info!(size = size, "AG-UI broadcast buffer size configured");
            }

        info!(
            agent_id = %config.agent_id,
            server_enabled = config.server.enabled,
            "A2A configuration loaded from secure config provider"
        );

        Ok(config)
    }

    /// DEPRECATED: Legacy environment loading method - use from_secure_config instead
    #[deprecated(note = "Use from_secure_config instead for better security")]
    pub fn from_environment() -> Self {
        warn!(
            "Using deprecated from_environment method - consider migrating to from_secure_config"
        );

        // Create a temporary secure config provider with env fallback enabled
        let config_provider = SecureConfigProvider::new().with_env_fallback(true);

        match Self::from_secure_config(&config_provider) {
            Ok(config) => config,
            Err(e) => {
                warn!(error = %e, "Failed to load config via secure provider, using defaults");
                Self::default()
            }
        }
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

    pub fn chat_delta_buffer_size(mut self, size: usize) -> Self {
        self.config.buffers.chat_delta_buffer_size = size;
        self
    }

    pub fn metrics_broadcast_buffer_size(mut self, size: usize) -> Self {
        self.config.buffers.metrics_broadcast_buffer_size = size;
        self
    }

    pub fn telemetry_channel_buffer_size(mut self, size: usize) -> Self {
        self.config.buffers.telemetry_channel_buffer_size = size;
        self
    }

    pub fn agui_broadcast_buffer_size(mut self, size: usize) -> Self {
        self.config.buffers.agui_broadcast_buffer_size = size;
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
    secure_provider: Option<SecureConfigProvider>,
}

impl ConfigManager {
    pub fn new(config: A2aConfig) -> Self {
        Self {
            config,
            secure_provider: None,
        }
    }

    /// Create config manager with secure config provider
    pub fn with_secure_provider(mut self, provider: SecureConfigProvider) -> Self {
        self.secure_provider = Some(provider);
        self
    }

    /// Create from secure config provider
    pub fn from_secure_config(provider: SecureConfigProvider) -> Result<Self> {
        let config = A2aConfig::from_secure_config(&provider)?;
        Ok(Self {
            config,
            secure_provider: Some(provider),
        })
    }

    /// DEPRECATED: Use from_secure_config instead
    #[deprecated(note = "Use from_secure_config instead for better security")]
    pub fn from_environment() -> Self {
        #[allow(deprecated)]
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

    /// Reload configuration from secure provider
    pub fn reload_from_secure_config(&mut self) -> Result<()> {
        if let Some(provider) = &self.secure_provider {
            self.config = A2aConfig::from_secure_config(provider)?;
            info!("Reloaded configuration from secure config provider");
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "No secure config provider available for reload"
            ))
        }
    }

    /// DEPRECATED: Use reload_from_secure_config instead
    #[deprecated(note = "Use reload_from_secure_config instead for better security")]
    pub fn reload_from_environment(&mut self) {
        warn!("Using deprecated reload_from_environment method");
        #[allow(deprecated)]
        {
            self.config = A2aConfig::from_environment();
        }
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

    #[test]
    fn test_secure_config_integration() {
        use arkavo_observability::config::{ConfigValidator, SecureConfigProvider};

        let mut provider = SecureConfigProvider::new();

        // Set up secure configuration values
        provider
            .set_config(
                "A2A_AGENT_ID",
                "secure-test-agent",
                ConfigValidator::required(),
            )
            .unwrap();

        provider
            .set_config(
                "A2A_SERVER_PORT",
                "9999",
                ConfigValidator::default().with_pattern(r"^\d+$"),
            )
            .unwrap();

        provider
            .set_config(
                "A2A_DISCOVERY_METHOD",
                "dns",
                ConfigValidator::default().with_pattern(r"^(static|dns)$"),
            )
            .unwrap();

        // Load config from secure provider
        let config = A2aConfig::from_secure_config(&provider).unwrap();

        assert_eq!(config.agent_id, "secure-test-agent");
        assert_eq!(config.server.port, 9999);
        assert!(config.server.enabled);
        assert!(matches!(config.discovery.method, DiscoveryMethod::Dns));
    }

    #[test]
    fn test_secure_config_manager() {
        use arkavo_observability::config::{ConfigValidator, SecureConfigProvider};

        let mut provider = SecureConfigProvider::new();
        provider
            .set_config(
                "A2A_AGENT_ID",
                "manager-test-agent",
                ConfigValidator::required(),
            )
            .unwrap();

        let manager = ConfigManager::from_secure_config(provider).unwrap();
        assert_eq!(manager.get().agent_id, "manager-test-agent");
    }
}
