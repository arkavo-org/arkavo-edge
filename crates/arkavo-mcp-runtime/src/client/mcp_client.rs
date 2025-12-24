use super::config::McpServerConfig;
use crate::ToolSchema;
use crate::transport::{Transport, TransportError, create_transport};
use serde_json::{Value, json};
use tracing::{debug, info};

/// MCP client for communicating with a single MCP server
pub struct McpClient {
    name: String,
    transport: Box<dyn Transport>,
    tools: Vec<ToolSchema>,
    server_info: Option<ServerInfo>,
}

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
}

impl McpClient {
    /// Create a new MCP client from configuration
    pub async fn connect(config: McpServerConfig) -> Result<Self, TransportError> {
        let transport = create_transport(config.transport, config.env).await?;

        let mut client = Self {
            name: config.name,
            transport,
            tools: Vec::new(),
            server_info: None,
        };

        // Perform initialization handshake
        client.initialize().await?;

        // Discover tools
        client.discover_tools().await?;

        Ok(client)
    }

    /// Initialize the MCP protocol handshake
    async fn initialize(&mut self) -> Result<(), TransportError> {
        let result = self
            .transport
            .send_request(
                "initialize",
                Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "roots": { "listChanged": true },
                        "sampling": {}
                    },
                    "clientInfo": {
                        "name": "arkavo-mcp-runtime",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
            )
            .await?;

        // Parse server info from response
        if let Some(server_info) = result.get("serverInfo") {
            self.server_info = Some(ServerInfo {
                name: server_info
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                version: server_info
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                protocol_version: result
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            });
        }

        // Send initialized notification
        self.transport
            .send_notification("notifications/initialized", None)
            .await?;

        info!(
            "MCP client '{}' initialized with server: {:?}",
            self.name, self.server_info
        );

        Ok(())
    }

    /// Discover available tools from the server
    async fn discover_tools(&mut self) -> Result<(), TransportError> {
        let result = self.transport.send_request("tools/list", None).await?;

        if let Some(tools) = result.get("tools").and_then(|v| v.as_array()) {
            for tool in tools {
                if let (Some(name), Some(description)) = (
                    tool.get("name").and_then(|v| v.as_str()),
                    tool.get("description").and_then(|v| v.as_str()),
                ) {
                    let parameters = tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

                    self.tools.push(ToolSchema {
                        name: name.to_string(),
                        aliases: None,
                        description: description.to_string(),
                        parameters,
                    });
                }
            }
        }

        debug!(
            "MCP client '{}' discovered {} tools",
            self.name,
            self.tools.len()
        );

        Ok(())
    }

    /// Get the client name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the discovered tools
    pub fn tools(&self) -> &[ToolSchema] {
        &self.tools
    }

    /// Get server info
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    /// Call a tool on the remote server
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, TransportError> {
        let result = self
            .transport
            .send_request(
                "tools/call",
                Some(json!({
                    "name": tool_name,
                    "arguments": arguments
                })),
            )
            .await?;

        // Extract content from response
        if let Some(content) = result.get("content") {
            // MCP returns an array of content blocks
            if let Some(blocks) = content.as_array() {
                if blocks.len() == 1 {
                    // Single block - return its text or content
                    if let Some(text) = blocks[0].get("text") {
                        return Ok(text.clone());
                    }
                    return Ok(blocks[0].clone());
                }
                return Ok(json!(blocks));
            }
            return Ok(content.clone());
        }

        Ok(result)
    }

    /// Check if the connection is still alive
    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }

    /// Close the connection
    pub async fn close(&self) -> Result<(), TransportError> {
        self.transport.close().await
    }
}
