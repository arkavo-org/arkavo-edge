use crate::client::{ClientHandle, McpClient, McpServerConfig};
use crate::server::McpServer;
use crate::transport::TransportError;
use crate::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Runtime for managing multiple MCP clients and a local server
pub struct McpRuntime {
    server: Arc<RwLock<McpServer>>,
    clients: Arc<RwLock<HashMap<String, Arc<McpClient>>>>,
    tool_origins: Arc<RwLock<HashMap<String, String>>>,
}

impl McpRuntime {
    /// Create a new MCP runtime with an empty server
    pub fn new() -> Self {
        Self {
            server: Arc::new(RwLock::new(McpServer::new())),
            clients: Arc::new(RwLock::new(HashMap::new())),
            tool_origins: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new MCP runtime with an existing server
    pub fn with_server(server: McpServer) -> Self {
        Self {
            server: Arc::new(RwLock::new(server)),
            clients: Arc::new(RwLock::new(HashMap::new())),
            tool_origins: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a new MCP server and register its tools
    pub async fn add_server(&self, config: McpServerConfig) -> Result<ClientHandle, RuntimeError> {
        let name = config.name.clone();

        // Check if already exists
        {
            let clients = self.clients.read().await;
            if clients.contains_key(&name) {
                return Err(RuntimeError::AlreadyExists(name));
            }
        }

        // Connect to the server
        let client = McpClient::connect(config)
            .await
            .map_err(RuntimeError::Transport)?;

        let client = Arc::new(client);

        // Register proxy tools for each tool the server provides
        let tools = client.tools().to_vec();
        {
            let server = self.server.write().await;
            let mut tool_origins = self.tool_origins.write().await;

            for tool_schema in &tools {
                // Check for conflicts - last server wins
                if let Some(old_origin) = tool_origins.get(&tool_schema.name) {
                    warn!(
                        "Tool '{}' from '{}' replacing tool from '{}'",
                        tool_schema.name, name, old_origin
                    );
                }

                let proxy = ProxyTool::new(tool_schema.clone(), client.clone());
                server
                    .register_or_replace_tool(tool_schema.name.clone(), Arc::new(proxy))
                    .await;
                tool_origins.insert(tool_schema.name.clone(), name.clone());
            }
        }

        // Store the client
        {
            let mut clients = self.clients.write().await;
            clients.insert(name.clone(), client);
        }

        info!("Added MCP server '{}' with {} tools", name, tools.len());

        Ok(ClientHandle::new(name))
    }

    /// Remove an MCP server and unregister its tools
    pub async fn remove_server(&self, name: &str) -> Result<(), RuntimeError> {
        // Remove the client
        let client = {
            let mut clients = self.clients.write().await;
            clients.remove(name)
        };

        let client = match client {
            Some(c) => c,
            None => return Err(RuntimeError::NotFound(name.to_string())),
        };

        // Close the connection
        if let Err(e) = client.close().await {
            warn!("Error closing client '{}': {}", name, e);
        }

        // Unregister tools that came from this server
        {
            let server = self.server.write().await;
            let mut tool_origins = self.tool_origins.write().await;

            let tools_to_remove: Vec<String> = tool_origins
                .iter()
                .filter(|(_, origin)| *origin == name)
                .map(|(tool, _)| tool.clone())
                .collect();

            for tool_name in tools_to_remove {
                server.unregister_tool(&tool_name).await;
                tool_origins.remove(&tool_name);
            }
        }

        info!("Removed MCP server '{}'", name);

        Ok(())
    }

    /// List all connected servers
    pub async fn list_servers(&self) -> Vec<String> {
        let clients = self.clients.read().await;
        clients.keys().cloned().collect()
    }

    /// Get the underlying MCP server
    pub async fn server(&self) -> tokio::sync::RwLockReadGuard<'_, McpServer> {
        self.server.read().await
    }

    /// Get a mutable reference to the underlying MCP server
    pub async fn server_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, McpServer> {
        self.server.write().await
    }

    /// Check if a server is connected
    pub async fn is_connected(&self, name: &str) -> bool {
        let clients = self.clients.read().await;
        clients.get(name).map(|c| c.is_connected()).unwrap_or(false)
    }

    /// Shutdown all connections
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let clients = {
            let mut clients = self.clients.write().await;
            std::mem::take(&mut *clients)
        };

        for (name, client) in clients {
            if let Err(e) = client.close().await {
                warn!("Error closing client '{}': {}", name, e);
            }
        }

        info!("MCP runtime shutdown complete");

        Ok(())
    }
}

impl Default for McpRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Proxy tool that forwards calls to a remote MCP server
struct ProxyTool {
    schema: ToolSchema,
    client: Arc<McpClient>,
}

impl ProxyTool {
    fn new(schema: ToolSchema, client: Arc<McpClient>) -> Self {
        Self { schema, client }
    }
}

impl std::fmt::Debug for ProxyTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyTool")
            .field("name", &self.schema.name)
            .field("client", &self.client.name())
            .finish()
    }
}

#[async_trait]
impl Tool for ProxyTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        debug!(
            "Proxying tool '{}' to server '{}'",
            self.schema.name,
            self.client.name()
        );

        self.client
            .call_tool(&self.schema.name, params)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

/// Runtime error types
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Server '{0}' already exists")]
    AlreadyExists(String),

    #[error("Server '{0}' not found")]
    NotFound(String),

    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
}
