//! MCP Runtime for Dynamic Server Management
//!
//! Provides a runtime for dynamically connecting to and managing multiple
//! MCP servers at runtime, supporting both Stdio and HTTP transports.

use crate::client::{ClientHandle, McpServerConfig, McpTransportConfig, ToolInfo};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Internal state for a connected MCP client
enum McpClientState {
    Stdio(StdioClient),
    Http(HttpClient),
}

/// Stdio-based MCP client (spawned process)
struct StdioClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: u64,
    tools: Vec<ToolInfo>,
}

/// HTTP-based MCP client (REST endpoints)
struct HttpClient {
    base_url: String,
    client: reqwest::Client,
    tools: Vec<ToolInfo>,
}

/// Runtime for managing dynamic MCP server connections
#[derive(Clone)]
pub struct McpRuntime {
    clients: Arc<RwLock<HashMap<String, McpClientState>>>,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: u64,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i32,
    message: String,
}

impl Default for McpRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl McpRuntime {
    /// Create a new MCP runtime
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a new MCP server connection
    pub async fn add_server(
        &self,
        config: McpServerConfig,
    ) -> Result<ClientHandle, Box<dyn std::error::Error + Send + Sync>> {
        let name = config.name.clone();

        // Check if already connected
        {
            let clients = self.clients.read().await;
            if clients.contains_key(&name) {
                return Err(format!("Server '{name}' is already connected").into());
            }
        }

        let (client_state, tools) = match config.transport {
            McpTransportConfig::Stdio { command, args } => Self::connect_stdio(&command, &args)?,
            McpTransportConfig::Http { url } => self.connect_http(&url).await?,
        };

        let handle = ClientHandle {
            name: name.clone(),
            tools: tools.clone(),
        };

        // Store the client
        {
            let mut clients = self.clients.write().await;
            clients.insert(name, client_state);
        }

        Ok(handle)
    }

    /// Connect to a Stdio-based MCP server
    fn connect_stdio(
        command: &str,
        args: &[String],
    ) -> Result<(McpClientState, Vec<ToolInfo>), Box<dyn std::error::Error + Send + Sync>> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start MCP server '{command}': {e}"))?;

        let mut stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
        let mut stdout_reader = BufReader::new(stdout);

        // Send initialize request
        let init_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "initialize".to_string(),
            params: Some(json!({
                "clientInfo": {
                    "name": "arkavo-mcp-runtime",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        };

        writeln!(&mut stdin, "{}", serde_json::to_string(&init_request)?)?;
        stdin.flush()?;

        // Read initialize response
        let mut init_line = String::new();
        stdout_reader.read_line(&mut init_line)?;

        if !init_line.is_empty() {
            let response: JsonRpcResponse = serde_json::from_str(&init_line)?;
            if let Some(error) = response.error {
                return Err(format!("MCP initialization failed: {}", error.message).into());
            }
        }

        // Send initialized notification
        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        writeln!(
            &mut stdin,
            "{}",
            serde_json::to_string(&initialized_notification)?
        )?;
        stdin.flush()?;

        // List tools
        let list_tools_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 2,
            method: "tools/list".to_string(),
            params: Some(json!({})),
        };

        writeln!(
            &mut stdin,
            "{}",
            serde_json::to_string(&list_tools_request)?
        )?;
        stdin.flush()?;

        let mut tools_line = String::new();
        stdout_reader.read_line(&mut tools_line)?;

        let tools = if !tools_line.is_empty() {
            let response: JsonRpcResponse = serde_json::from_str(&tools_line)?;
            if let Some(result) = response.result {
                if let Some(tools_value) = result.get("tools") {
                    serde_json::from_value(tools_value.clone()).unwrap_or_default()
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let client = StdioClient {
            child,
            stdin,
            stdout: stdout_reader,
            request_id: 3, // Next request ID after init and tools/list
            tools: tools.clone(),
        };

        Ok((McpClientState::Stdio(client), tools))
    }

    /// Connect to an HTTP-based MCP server (REST endpoints)
    async fn connect_http(
        &self,
        base_url: &str,
    ) -> Result<(McpClientState, Vec<ToolInfo>), Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::new();

        // Check server health
        let health_url = format!("{}/health", base_url.trim_end_matches('/'));
        client
            .get(&health_url)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to HTTP server at {base_url}: {e}"))?;

        // For HTTP servers, we define tools based on known endpoints
        // The fleet-env server has: get_sector, inject_hazard
        let tools = vec![
            ToolInfo {
                name: "get_sector".to_string(),
                description: "Get information about a warehouse sector, including any hazards"
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Sector ID (1-based)"
                        }
                    },
                    "required": ["id"]
                }),
            },
            ToolInfo {
                name: "inject_hazard".to_string(),
                description: "Inject a hazard into a sector (for testing)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "sector": {
                            "type": "integer",
                            "description": "Sector ID to inject hazard"
                        },
                        "hazard_type": {
                            "type": "string",
                            "description": "Type of hazard (e.g., 'black_ice', 'oil_spill')"
                        },
                        "traction": {
                            "type": "number",
                            "description": "Traction coefficient (0.0-1.0)"
                        }
                    },
                    "required": ["sector", "hazard_type", "traction"]
                }),
            },
        ];

        let http_client = HttpClient {
            base_url: base_url.to_string(),
            client,
            tools: tools.clone(),
        };

        Ok((McpClientState::Http(http_client), tools))
    }

    /// Remove an MCP server connection
    pub async fn remove_server(
        &self,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut clients = self.clients.write().await;

        match clients.remove(name) {
            Some(client_state) => {
                // Graceful shutdown
                if let McpClientState::Stdio(mut stdio_client) = client_state {
                    let _ = stdio_client.child.kill();
                }
                Ok(())
            }
            None => Err(format!("Server '{name}' not found").into()),
        }
    }

    /// List all connected server names
    pub async fn list_servers(&self) -> Vec<String> {
        let clients = self.clients.read().await;
        clients.keys().cloned().collect()
    }

    /// Get tools from a specific server
    pub async fn get_tools(&self, server: &str) -> Option<Vec<ToolInfo>> {
        let clients = self.clients.read().await;
        clients.get(server).map(|c| match c {
            McpClientState::Stdio(s) => s.tools.clone(),
            McpClientState::Http(h) => h.tools.clone(),
        })
    }

    /// Get all tools from all connected servers
    pub async fn all_tools(&self) -> HashMap<String, Vec<ToolInfo>> {
        let clients = self.clients.read().await;
        clients
            .iter()
            .map(|(name, client)| {
                let tools = match client {
                    McpClientState::Stdio(s) => s.tools.clone(),
                    McpClientState::Http(h) => h.tools.clone(),
                };
                (name.clone(), tools)
            })
            .collect()
    }

    /// Call a tool on a specific server
    #[allow(clippy::significant_drop_tightening)]
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        args: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let mut clients = self.clients.write().await;
        let client = clients
            .get_mut(server)
            .ok_or_else(|| format!("Server '{server}' not found"))?;

        match client {
            McpClientState::Stdio(stdio_client) => Self::call_stdio_tool(stdio_client, tool, args),
            McpClientState::Http(http_client) => {
                Self::call_http_tool(http_client, tool, args).await
            }
        }
    }

    /// Call a tool via Stdio transport
    fn call_stdio_tool(
        client: &mut StdioClient,
        tool: &str,
        args: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: client.request_id,
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": tool,
                "arguments": args
            })),
        };
        client.request_id += 1;

        writeln!(client.stdin, "{}", serde_json::to_string(&request)?)?;
        client.stdin.flush()?;

        let mut line = String::new();
        client.stdout.read_line(&mut line)?;

        if line.is_empty() {
            return Err("No response from MCP server".into());
        }

        let response: JsonRpcResponse = serde_json::from_str(&line)?;

        if let Some(error) = response.error {
            return Err(format!("Tool execution error: {}", error.message).into());
        }

        // Extract result content
        if let Some(result) = response.result {
            if let Some(content_array) = result.get("content").and_then(|c| c.as_array())
                && let Some(first_content) = content_array.first()
                && let Some(text) = first_content.get("text").and_then(|t| t.as_str())
            {
                return Ok(json!({ "result": text }));
            }
            Ok(result)
        } else {
            Ok(json!({}))
        }
    }

    /// Call a tool via HTTP transport
    async fn call_http_tool(
        client: &HttpClient,
        tool: &str,
        args: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let base_url = client.base_url.trim_end_matches('/');

        match tool {
            "get_sector" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing 'id' parameter")?;

                let url = format!("{base_url}/sector/{id}");
                let response = client.client.get(&url).send().await?;

                if response.status().is_success() {
                    let result: Value = response.json().await?;
                    Ok(result)
                } else {
                    Err(format!("HTTP error: {}", response.status()).into())
                }
            }
            "inject_hazard" => {
                let url = format!("{base_url}/hazard");
                let response = client.client.post(&url).json(&args).send().await?;

                if response.status().is_success() {
                    let result: Value = response.json().await?;
                    Ok(result)
                } else {
                    Err(format!("HTTP error: {}", response.status()).into())
                }
            }
            _ => Err(format!("Unknown tool: {tool}").into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_runtime_new() {
        let runtime = McpRuntime::new();
        let servers = runtime.list_servers().await;
        assert!(servers.is_empty());
    }

    #[tokio::test]
    async fn test_remove_nonexistent_server() {
        let runtime = McpRuntime::new();
        let result = runtime.remove_server("nonexistent").await;
        assert!(result.is_err());
    }
}
