use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Agent configuration parsed from AGENTS.md
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub purpose: String,
    pub model: String,
    pub listen: String,
    pub mdns_enabled: bool,
    pub mcp_servers: Vec<McpServerConfig>,
    pub api_keys: HashMap<String, String>,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}

/// Parse AGENTS.md configuration content
pub fn parse_agents_config(content: &str) -> Result<Vec<AgentConfig>, Box<dyn std::error::Error>> {
    let mut agents = Vec::new();
    let mut current_agent: Option<AgentConfig> = None;
    let mut in_agent_section = false;
    let mut in_mcp_section = false;
    let mut current_mcp_server: Option<McpServerConfig> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for agent section header
        if trimmed.starts_with("## ") {
            // Save any pending MCP server before switching agents
            if let Some(server) = current_mcp_server.take()
                && let Some(agent) = current_agent.as_mut()
            {
                agent.mcp_servers.push(server);
            }

            // Save previous agent if exists
            if let Some(agent) = current_agent.take() {
                agents.push(agent);
            }

            // Reset MCP section flag
            in_mcp_section = false;

            let name = trimmed.strip_prefix("## ").unwrap_or("").trim().to_string();
            current_agent = Some(AgentConfig {
                name,
                purpose: String::new(),
                model: String::new(),
                listen: String::new(),
                mdns_enabled: true, // Default to true for zero-config
                mcp_servers: Vec::new(),
                api_keys: HashMap::new(),
                runtime: RuntimeConfig::default(),
            });
            in_agent_section = true;
            continue;
        }

        // Skip if not in agent section
        if !in_agent_section || current_agent.is_none() {
            continue;
        }

        // Check for mcp_servers section
        if trimmed == "mcp_servers:" {
            in_mcp_section = true;
            continue;
        }

        // Handle MCP server entries
        if in_mcp_section && trimmed.starts_with("- name:") {
            // Save previous MCP server if exists
            if let Some(server) = current_mcp_server.take()
                && let Some(agent) = current_agent.as_mut()
            {
                agent.mcp_servers.push(server);
            }

            // Start new MCP server
            current_mcp_server = Some(McpServerConfig {
                name: trimmed
                    .strip_prefix("- name:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                command: None,
                args: Vec::new(),
                url: None,
            });
            continue;
        }

        // Parse MCP server properties
        if in_mcp_section
            && current_mcp_server.is_some()
            && let Some(server) = current_mcp_server.as_mut()
        {
            if trimmed.starts_with("command:") {
                server.command = Some(
                    trimmed
                        .strip_prefix("command:")
                        .unwrap_or("")
                        .trim()
                        .trim_matches('"')
                        .to_string(),
                );
            } else if trimmed.starts_with("args:") {
                // Parse array format: ["arg1", "arg2"]
                let args_str = trimmed.strip_prefix("args:").unwrap_or("").trim();
                if args_str.starts_with('[') && args_str.ends_with(']') {
                    let args_content = &args_str[1..args_str.len() - 1];
                    server.args = args_content
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            } else if trimmed.starts_with("url:") {
                server.url = Some(
                    trimmed
                        .strip_prefix("url:")
                        .unwrap_or("")
                        .trim()
                        .trim_matches('"')
                        .to_string(),
                );
            } else if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('-')
            {
                // End of MCP section
                in_mcp_section = false;
                if let Some(server) = current_mcp_server.take()
                    && let Some(agent) = current_agent.as_mut()
                {
                    agent.mcp_servers.push(server);
                }
            }
        }

        // Parse agent properties (when not in MCP section)
        if !in_mcp_section && let Some(agent) = current_agent.as_mut() {
            if trimmed.starts_with("purpose:") {
                agent.purpose = trimmed
                    .strip_prefix("purpose:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            } else if trimmed.starts_with("model:") {
                agent.model = trimmed
                    .strip_prefix("model:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            } else if trimmed.starts_with("listen:") {
                agent.listen = trimmed
                    .strip_prefix("listen:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            } else if trimmed.starts_with("mdns:") {
                // Only disable if explicitly set to false
                agent.mdns_enabled = !trimmed.contains("false");
            } else if trimmed.contains("_API_KEY:") || trimmed.contains("_api_key:") {
                // Parse API key entries (e.g., MOONSHOT_API_KEY: sk-xxx)
                if let Some(colon_pos) = trimmed.find(':') {
                    let key_name = trimmed[..colon_pos].trim().to_string();
                    let key_value = trimmed[colon_pos + 1..]
                        .trim()
                        .trim_matches('"')
                        .to_string();
                    agent.api_keys.insert(key_name, key_value);
                }
            }
        }
    }

    // Save any pending MCP server
    if let Some(server) = current_mcp_server
        && let Some(agent) = current_agent.as_mut()
    {
        agent.mcp_servers.push(server);
    }

    // Save last agent
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}

/// Workspace paths parsed from AGENTS.md paths: section
#[derive(Debug, Clone, Default)]
pub struct WorkspacePaths {
    pub memory_db_path: Option<PathBuf>,
    pub workspace_root: PathBuf,
}

/// Parse workspace paths from AGENTS.md content.
/// Relative paths are resolved against the workspace_root (parent of .arkavo directory).
pub fn parse_workspace_paths(content: &str, workspace_root: &Path) -> WorkspacePaths {
    let mut paths = WorkspacePaths {
        memory_db_path: None,
        workspace_root: workspace_root.to_path_buf(),
    };

    let mut in_paths_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for Paths section header
        if trimmed == "## Paths" {
            in_paths_section = true;
            continue;
        }

        // End paths section on next ## header
        if in_paths_section && trimmed.starts_with("## ") {
            break;
        }

        // Parse paths: section marker
        if trimmed == "paths:" {
            in_paths_section = true;
            continue;
        }

        // Parse memory_db path
        if in_paths_section && trimmed.starts_with("memory_db:") {
            let path_str = trimmed
                .strip_prefix("memory_db:")
                .unwrap_or("")
                .trim()
                .trim_matches('"');

            if !path_str.is_empty() {
                let path = PathBuf::from(path_str);
                // Resolve relative paths against workspace root
                let resolved = if path.is_absolute() {
                    path
                } else {
                    workspace_root.join(path)
                };
                paths.memory_db_path = Some(resolved);
            }
        }
    }

    paths
}

/// Parse runtime configuration from AGENTS.md content.
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
    fn test_parse_workspace_paths_relative() {
        let content = r#"
## Paths

paths:
  memory_db: .arkavo/memory_server/memories.db
"#;
        let workspace_root = PathBuf::from("/home/user/project");
        let paths = parse_workspace_paths(content, &workspace_root);

        assert_eq!(
            paths.memory_db_path,
            Some(PathBuf::from(
                "/home/user/project/.arkavo/memory_server/memories.db"
            ))
        );
        assert_eq!(paths.workspace_root, workspace_root);
    }

    #[test]
    fn test_parse_workspace_paths_absolute() {
        let content = r#"
## Paths

paths:
  memory_db: /absolute/path/to/memories.db
"#;
        let workspace_root = PathBuf::from("/home/user/project");
        let paths = parse_workspace_paths(content, &workspace_root);

        assert_eq!(
            paths.memory_db_path,
            Some(PathBuf::from("/absolute/path/to/memories.db"))
        );
    }

    #[test]
    fn test_parse_workspace_paths_no_paths_section() {
        let content = r#"
## Agent

name: test-agent
model: gpt-4
"#;
        let workspace_root = PathBuf::from("/home/user/project");
        let paths = parse_workspace_paths(content, &workspace_root);

        assert_eq!(paths.memory_db_path, None);
    }

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
