use std::collections::HashMap;

/// Runtime configuration for timeouts and limits
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Timeout for mDNS agent discovery in seconds (default: 3)
    pub mdns_discovery_timeout_secs: u64,
    /// Timeout for A2A transport requests in milliseconds (default: 60000)
    pub transport_timeout_ms: u64,
    /// Maximum time to wait for task completion in seconds (default: 300)
    pub task_execution_timeout_secs: u64,
    /// Interval between task status polls in seconds (default: 2)
    pub poll_interval_secs: u64,
    /// Maximum SQLite pool connections (default: 5)
    pub max_pool_connections: u32,
    /// Similarity threshold for memory categorization (default: 0.3)
    pub categorization_threshold: f32,
    /// Maximum vectors in hot tier (default: 10000)
    pub max_hot_vectors: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            mdns_discovery_timeout_secs: 3,
            transport_timeout_ms: 60_000,
            task_execution_timeout_secs: 300,
            poll_interval_secs: 2,
            max_pool_connections: 5,
            categorization_threshold: 0.3,
            max_hot_vectors: 10_000,
        }
    }
}

/// Agent configuration. Historically parsed from AGENTS.md; that parser was
/// deleted in Task 14 / S6 (SwarmKit is now the sole config source) — this
/// type itself remains live, used across arkavo-server.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub purpose: String,
    pub model: String,
    pub mode: AgentMode,
    pub listen: String,
    pub mdns_enabled: bool,
    pub mcp_servers: Vec<McpServerConfig>,
    pub api_keys: HashMap<String, String>,
    pub runtime: RuntimeConfig,
}

/// Agent execution mode.
/// Orchestrator: autonomous tick loop (observe → plan → act).
/// Specialist: passive, only responds to message/send tasks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AgentMode {
    #[default]
    Orchestrator,
    Specialist,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}
