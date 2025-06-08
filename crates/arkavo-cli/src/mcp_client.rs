use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug, Clone)]
pub struct McpClient {
    process: Arc<Mutex<McpProcess>>,
    request_id: Arc<Mutex<u64>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i32,
    message: String,
    #[allow(dead_code)]
    data: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallMetadata {
    pub timestamp: u64,
    pub llm_origin: String,
    pub tool_name: String,
    pub arguments: Value,
    pub request_id: u64,
}

impl McpClient {
    pub fn new(mcp_url: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        // If URL provided, parse it to extract command and args
        let (cmd, args) = if let Some(url) = mcp_url {
            // For now, support simple command URLs like "arkavo mcp"
            let parts: Vec<String> = url.split_whitespace().map(|s| s.to_string()).collect();
            if parts.is_empty() {
                return Err("Invalid MCP URL".into());
            }
            (parts[0].clone(), parts[1..].to_vec())
        } else {
            // Default to local arkavo mcp
            ("arkavo".to_string(), vec!["mcp".to_string()])
        };

        // Start MCP server process
        let mut child = Command::new(&cmd)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start MCP server '{}': {}", cmd, e))?;

        // Take ownership of stdin and stdout
        let mut stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
        let stdout_reader = BufReader::new(stdout);

        let init_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "initialize".to_string(),
            params: Some(json!({
                "clientInfo": {
                    "name": "arkavo-chat",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        };

        // Send initialize request
        writeln!(&mut stdin, "{}", serde_json::to_string(&init_request)?)?;
        stdin.flush()?;

        // Create a mutable reference to read the response
        let mut reader = stdout_reader;
        let mut init_line = String::new();
        reader.read_line(&mut init_line)?;

        if !init_line.is_empty() {
            let response: JsonRpcResponse = serde_json::from_str(&init_line)?;
            if let Some(error) = response.error {
                return Err(format!("MCP initialization failed: {}", error.message).into());
            }
            // Removed large debug output
        }

        // Send initialized notification
        let initialized_notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        writeln!(&mut stdin, "{}", serde_json::to_string(&initialized_notification)?)?;
        stdin.flush()?;

        // Create the process wrapper with persistent stdin/stdout
        let mcp_process = McpProcess {
            child,
            stdin,
            stdout: reader,
        };

        Ok(Self {
            process: Arc::new(Mutex::new(mcp_process)),
            request_id: Arc::new(Mutex::new(2)), // Start from 2 since we used 1 for init
        })
    }

    pub fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        // The tools are returned in the initialize response, let's request them again
        let response = self.send_request("tools/list", Some(json!({})))?;

        if let Some(result) = response.result {
            // Check for tools in different possible locations
            if let Some(tools_value) = result.get("tools") {
                let tools: Vec<Tool> = serde_json::from_value(tools_value.clone())?;
                Ok(tools)
            } else if let Some(tools_value) = result
                .get("capabilities")
                .and_then(|c| c.get("tools"))
                .and_then(|t| t.get("available"))
            {
                let tools: Vec<Tool> = serde_json::from_value(tools_value.clone())?;
                Ok(tools)
            } else {
                // If not found, return empty list
                Ok(vec![])
            }
        } else {
            Ok(vec![])
        }
    }

    pub fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        llm_origin: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        // Create metadata for logging
        let metadata = ToolCallMetadata {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            llm_origin: llm_origin.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            request_id: *self.request_id.lock().unwrap(),
        };

        // Log the tool call
        eprintln!(
            "[MCP Tool Call] {} | LLM: {} | Tool: {} | Args: {}",
            metadata.timestamp,
            metadata.llm_origin,
            metadata.tool_name,
            serde_json::to_string(&metadata.arguments)?
        );

        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", Some(params))?;

        if let Some(error) = response.error {
            return Err(format!("Tool execution error: {}", error.message).into());
        }

        // Extract the text content from the MCP response format
        if let Some(result) = response.result {
            if let Some(content_array) = result.get("content").and_then(|c| c.as_array()) {
                if let Some(first_content) = content_array.first() {
                    if let Some(text) = first_content.get("text").and_then(|t| t.as_str()) {
                        return Ok(json!({ "result": text }));
                    }
                }
            }
            Ok(result)
        } else {
            Ok(json!({}))
        }
    }

    fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, Box<dyn std::error::Error>> {
        let mut process = self.process.lock().unwrap();

        // Get next request ID
        let request_id = {
            let mut id = self.request_id.lock().unwrap();
            let current = *id;
            *id += 1;
            current
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: request_id,
            method: method.to_string(),
            params,
        };

        // Send request
        writeln!(process.stdin, "{}", serde_json::to_string(&request)?)?;
        process.stdin.flush()?;

        // Read response
        let mut line = String::new();
        process.stdout.read_line(&mut line)?;

        if !line.is_empty() {
            let response: JsonRpcResponse = serde_json::from_str(&line)?;
            Ok(response)
        } else {
            Err("No response from MCP server".into())
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.lock() {
            let _ = process.child.kill();
        }
    }
}
