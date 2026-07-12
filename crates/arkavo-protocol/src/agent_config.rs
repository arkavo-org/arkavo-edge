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

/// Parse runtime configuration from a runtime: block.
///
/// The sole remaining caller (Task 14 / S6 audit) is
/// `arkavo-protocol/tests/sequence_integrity_test.rs`'s SEQ-016 tripwire,
/// which calls this with an empty string purely to obtain a
/// [`RuntimeConfig`] value to inspect — it is not an AGENTS.md-reading test.
/// Kept rather than deleted per the migration brief's "leave a live caller"
/// rule; safe to delete once that tripwire is rewritten against
/// `RuntimeConfig::default()` directly.
///
/// Example format:
/// ```yaml
/// runtime:
///   mdns_discovery_timeout_secs: 5
///   transport_timeout_ms: 30000
///   task_execution_timeout_secs: 600
///   poll_interval_secs: 3
///   max_pool_connections: 10
///   categorization_threshold: 0.4
///   max_hot_vectors: 20000
/// ```
pub fn parse_runtime_config(content: &str) -> RuntimeConfig {
    let mut config = RuntimeConfig::default();
    let mut in_runtime_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for runtime section header
        if trimmed == "runtime:" {
            in_runtime_section = true;
            continue;
        }

        // End runtime section on non-indented line (except empty)
        if in_runtime_section
            && !trimmed.is_empty()
            && !line.starts_with(' ')
            && !line.starts_with('\t')
        {
            break;
        }

        // Parse runtime values
        if in_runtime_section {
            if let Some(val) = trimmed.strip_prefix("mdns_discovery_timeout_secs:") {
                if let Ok(v) = val.trim().parse() {
                    config.mdns_discovery_timeout_secs = v;
                }
            } else if let Some(val) = trimmed.strip_prefix("transport_timeout_ms:") {
                if let Ok(v) = val.trim().parse() {
                    config.transport_timeout_ms = v;
                }
            } else if let Some(val) = trimmed.strip_prefix("task_execution_timeout_secs:") {
                if let Ok(v) = val.trim().parse() {
                    config.task_execution_timeout_secs = v;
                }
            } else if let Some(val) = trimmed.strip_prefix("poll_interval_secs:") {
                if let Ok(v) = val.trim().parse() {
                    config.poll_interval_secs = v;
                }
            } else if let Some(val) = trimmed.strip_prefix("max_pool_connections:") {
                if let Ok(v) = val.trim().parse() {
                    config.max_pool_connections = v;
                }
            } else if let Some(val) = trimmed.strip_prefix("categorization_threshold:") {
                if let Ok(v) = val.trim().parse::<f32>() {
                    config.categorization_threshold = v.clamp(0.0, 1.0);
                }
            } else if let Some(val) = trimmed.strip_prefix("max_hot_vectors:")
                && let Ok(v) = val.trim().parse()
            {
                config.max_hot_vectors = v;
            }
        }
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_runtime_config_with_values() {
        let content = r#"
## Runtime Configuration

runtime:
  mdns_discovery_timeout_secs: 5
  transport_timeout_ms: 30000
  task_execution_timeout_secs: 600
  poll_interval_secs: 3
  max_pool_connections: 10
  categorization_threshold: 0.4
  max_hot_vectors: 20000
"#;
        let config = parse_runtime_config(content);

        assert_eq!(config.mdns_discovery_timeout_secs, 5);
        assert_eq!(config.transport_timeout_ms, 30000);
        assert_eq!(config.task_execution_timeout_secs, 600);
        assert_eq!(config.poll_interval_secs, 3);
        assert_eq!(config.max_pool_connections, 10);
        assert!((config.categorization_threshold - 0.4).abs() < 0.001);
        assert_eq!(config.max_hot_vectors, 20000);
    }

    #[test]
    fn test_parse_runtime_config_defaults() {
        let content = r#"
## Agent

name: test-agent
"#;
        let config = parse_runtime_config(content);

        assert_eq!(config.mdns_discovery_timeout_secs, 3);
        assert_eq!(config.transport_timeout_ms, 60_000);
        assert_eq!(config.task_execution_timeout_secs, 300);
        assert_eq!(config.poll_interval_secs, 2);
        assert_eq!(config.max_pool_connections, 5);
        assert!((config.categorization_threshold - 0.3).abs() < 0.001);
        assert_eq!(config.max_hot_vectors, 10_000);
    }

    #[test]
    fn test_parse_runtime_config_partial() {
        let content = r#"
runtime:
  mdns_discovery_timeout_secs: 10
  max_hot_vectors: 5000
"#;
        let config = parse_runtime_config(content);

        assert_eq!(config.mdns_discovery_timeout_secs, 10);
        assert_eq!(config.max_hot_vectors, 5000);
        // Other values should remain default
        assert_eq!(config.transport_timeout_ms, 60_000);
    }
}
