//! MCP Client Configuration Types
//!
//! Provides types for configuring MCP server connections,
//! supporting both Stdio and HTTP transports.

use serde::{Deserialize, Serialize};

/// Configuration for connecting to an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server connection
    pub name: String,
    /// Transport configuration
    pub transport: McpTransportConfig,
}

/// Transport-specific configuration for MCP connections
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpTransportConfig {
    /// Stdio transport - spawns a child process
    Stdio {
        /// Command to execute
        command: String,
        /// Arguments for the command
        #[serde(default)]
        args: Vec<String>,
    },
    /// HTTP transport - connects to REST endpoints
    Http {
        /// Base URL for the HTTP server
        url: String,
    },
}

impl McpServerConfig {
    /// Create a new Stdio-based MCP server config
    pub fn stdio(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            transport: McpTransportConfig::Stdio {
                command: command.into(),
                args,
            },
        }
    }

    /// Create a new HTTP-based MCP server config
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: McpTransportConfig::Http { url: url.into() },
        }
    }
}

/// Handle to a connected MCP server
#[derive(Debug, Clone)]
pub struct ClientHandle {
    /// Name of the connected server
    pub name: String,
    /// Available tools from this server
    pub tools: Vec<ToolInfo>,
}

/// Information about a tool available from an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// JSON schema for the tool's input parameters
    pub input_schema: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdio_config() {
        let config = McpServerConfig::stdio("test", "/usr/bin/mcp", vec!["--arg".to_string()]);
        assert_eq!(config.name, "test");
        match config.transport {
            McpTransportConfig::Stdio { command, args } => {
                assert_eq!(command, "/usr/bin/mcp");
                assert_eq!(args, vec!["--arg"]);
            }
            _ => panic!("Expected Stdio transport"),
        }
    }

    #[test]
    fn test_http_config() {
        let config = McpServerConfig::http("fleet-env", "http://localhost:8360");
        assert_eq!(config.name, "fleet-env");
        match config.transport {
            McpTransportConfig::Http { url } => {
                assert_eq!(url, "http://localhost:8360");
            }
            _ => panic!("Expected Http transport"),
        }
    }

    #[test]
    fn test_config_serialization() {
        let config = McpServerConfig::http("test", "http://localhost:8080");
        let json = serde_json::to_string(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
    }
}
