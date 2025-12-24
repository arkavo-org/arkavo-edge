use crate::transport::TransportConfig;
use std::collections::HashMap;

/// Configuration for an MCP server connection
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Unique name for this server
    pub name: String,
    /// Transport configuration
    pub transport: TransportConfig,
    /// Environment variables to pass to the server (for Stdio transport)
    pub env: HashMap<String, String>,
    /// Timeout for requests in milliseconds
    pub timeout_ms: u64,
}

impl McpServerConfig {
    /// Create a new MCP server config with Stdio transport
    pub fn stdio(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            transport: TransportConfig::Stdio {
                command: command.into(),
                args,
                cwd: None,
            },
            env: HashMap::new(),
            timeout_ms: 30000,
        }
    }

    /// Create a new MCP server config with SSE transport
    pub fn sse(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: TransportConfig::Sse {
                url: url.into(),
                headers: HashMap::new(),
            },
            env: HashMap::new(),
            timeout_ms: 30000,
        }
    }

    /// Create a new MCP server config with WebSocket transport
    pub fn websocket(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: TransportConfig::WebSocket {
                url: url.into(),
                headers: HashMap::new(),
            },
            env: HashMap::new(),
            timeout_ms: 30000,
        }
    }

    /// Set the working directory (for Stdio transport)
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        if let TransportConfig::Stdio { cwd: ref mut c, .. } = self.transport {
            *c = Some(cwd.into());
        }
        self
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Add a header (for SSE/WebSocket transports)
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        match &mut self.transport {
            TransportConfig::Sse { headers, .. } | TransportConfig::WebSocket { headers, .. } => {
                headers.insert(key.into(), value.into());
            }
            _ => {}
        }
        self
    }

    /// Set the timeout in milliseconds
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}
