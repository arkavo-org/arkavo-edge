use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct McpClient {
    process: Arc<Mutex<Child>>,
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
    jsonrpc: String,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
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

        // Initialize the MCP server
        let stdin = child.stdin.as_mut().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;

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
        writeln!(stdin, "{}", serde_json::to_string(&init_request)?)?;
        stdin.flush()?;

        // Read initialize response
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        if let Some(Ok(line)) = lines.next() {
            let response: JsonRpcResponse = serde_json::from_str(&line)?;
            if let Some(error) = response.error {
                return Err(format!("MCP initialization failed: {}", error.message).into());
            }
            eprintln!("MCP server initialized: {:?}", response.result);
        }
        drop(lines); // Drop lines iterator to release reader

        // Note: stdout is consumed by BufReader, process communication will happen per request

        Ok(Self {
            process: Arc::new(Mutex::new(child)),
            request_id: Arc::new(Mutex::new(2)), // Start from 2 since we used 1 for init
        })
    }

    pub fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        let response = self.send_request("tools/list", None)?;
        
        if let Some(result) = response.result {
            if let Some(tools_value) = result.get("tools") {
                let tools: Vec<Tool> = serde_json::from_value(tools_value.clone())?;
                Ok(tools)
            } else {
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
        if let Some(stdin) = process.stdin.as_mut() {
            writeln!(stdin, "{}", serde_json::to_string(&request)?)?;
            stdin.flush()?;
        } else {
            return Err("Failed to get stdin".into());
        }

        // Read response line by line
        let mut line = String::new();
        use std::io::BufRead;
        
        if let Some(stdout) = process.stdout.as_mut() {
            let mut reader = BufReader::new(stdout);
            reader.read_line(&mut line)?;
        } else {
            return Err("Failed to get stdout".into());
        }
        
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
            let _ = process.kill();
        }
    }
}