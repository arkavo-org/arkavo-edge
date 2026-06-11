use arkavo_mcp_runtime::polling::{
    AdaptiveBackoff, PollConfig, PollResultParams, PollableEndpoint,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, oneshot, watch};

fn debug_mcp() -> bool {
    std::env::var("ARKAVO_DEBUG").is_ok() || std::env::var("ARKAVO_DEBUG_MCP").is_ok()
}

fn log_mcp(msg: &str) {
    if debug_mcp() {
        eprintln!("[MCP] {msg}");
    }
}

/// Dual-mode pending request - supports both sync (mpsc) and async (oneshot)
enum PendingRequest {
    Sync(mpsc::Sender<JsonRpcResponse>),
    Async(oneshot::Sender<JsonRpcResponse>),
}

/// Type alias for pending request senders
type PendingRequests = Arc<Mutex<HashMap<u64, PendingRequest>>>;

/// MCP client for communicating with MCP servers via subprocess
#[derive(Clone)]
pub struct McpClient {
    stdin: Arc<Mutex<ChildStdin>>,
    request_id: Arc<Mutex<u64>>,
    pending_requests: PendingRequests,
    notification_tx: broadcast::Sender<JsonRpcNotification>,
    child: Arc<Mutex<Child>>,
    // Polling support
    poll_endpoints: Arc<Mutex<Vec<PollableEndpoint>>>,
    poll_shutdown_tx: Arc<Mutex<Option<watch::Sender<bool>>>>,
    server_name: String,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("server_name", &self.server_name)
            .field(
                "poll_endpoints_count",
                &self.poll_endpoints.lock().map(|e| e.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Option<Value>,
}

/// Response from MCP server
#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: u64,
    /// The result value if successful
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error information if request failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// Error from MCP server
#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Optional additional data
    pub data: Option<Value>,
}

/// Notification from MCP server (no id field)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Generic message that can be either a response or notification
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonRpcMessage {
    Response {
        #[allow(dead_code)]
        jsonrpc: String,
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<JsonRpcError>,
    },
    Notification {
        #[allow(dead_code)]
        jsonrpc: String,
        method: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Value>,
    },
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
            let parts: Vec<String> = url
                .split_whitespace()
                .map(std::string::ToString::to_string)
                .collect();
            if parts.is_empty() {
                return Err("Invalid MCP URL".into());
            }
            (parts[0].clone(), parts[1..].to_vec())
        } else {
            // Default to local arkavo mcp
            // Use development binary if available, otherwise fall back to system
            let dev_binary = "./target/debug/arkavo";
            let cmd = if std::path::Path::new(dev_binary).exists() {
                dev_binary.to_string()
            } else {
                "arkavo".to_string()
            };
            (cmd, vec!["mcp".to_string()])
        };

        Self::new_with_command(&cmd, &args)
    }

    /// Create a new MCP client with explicit command and arguments
    pub fn new_with_command(
        cmd: &str,
        args: &[String],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Expand ${VAR} at the spawn site so every config parse path
        // (frontmatter, markdown format, protocol crate) benefits.
        let cmd = arkavo_protocol::agent_config::expand_env(cmd);
        let cmd = cmd.as_str();
        let args: Vec<String> = args
            .iter()
            .map(|a| arkavo_protocol::agent_config::expand_env(a))
            .collect();
        log_mcp(&format!("Starting: {cmd} {}", args.join(" ")));

        // Start MCP server process
        let mut child = Command::new(cmd)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start MCP server '{cmd}': {e}"))?;

        let pid = child.id();
        log_mcp(&format!("Subprocess PID: {pid}"));

        // Spawn stderr reader thread to capture MCP server errors
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line_result in reader.lines() {
                    match line_result {
                        Ok(line) => eprintln!("[MCP-STDERR] {line}"),
                        Err(_) => break,
                    }
                }
            });
        }

        // Take ownership of stdin and stdout
        let mut stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
        let mut stdout_reader = BufReader::new(stdout);

        // Initialize handshake (synchronous, before background reader starts)
        let init_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "initialize".to_string(),
            params: Some(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {
                    "roots": { "listChanged": true },
                    "sampling": {}
                },
                "clientInfo": {
                    "name": "arkavo-cli",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        };

        log_mcp("Sending initialize request");
        writeln!(&mut stdin, "{}", serde_json::to_string(&init_request)?)?;
        stdin.flush()?;

        let mut init_line = String::new();
        stdout_reader.read_line(&mut init_line)?;

        if !init_line.is_empty() {
            log_mcp(&format!("Initialize response: {}", init_line.trim()));
            let response: JsonRpcResponse = serde_json::from_str(&init_line)?;
            if let Some(error) = response.error {
                log_mcp(&format!("Initialize FAILED: {}", error.message));
                return Err(format!("MCP initialization failed: {}", error.message).into());
            }
            log_mcp("Initialize succeeded");
        } else {
            log_mcp("WARNING: Empty initialize response");
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

        // Set up channels for async communication
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (notification_tx, _) = broadcast::channel(256);

        // Spawn background reader thread for push-based notifications
        let pending_clone = pending_requests.clone();
        let notif_tx_clone = notification_tx.clone();
        std::thread::spawn(move || {
            Self::reader_loop(stdout_reader, pending_clone, notif_tx_clone);
        });

        // Extract server name from command for poll notifications
        let server_name = cmd.split('/').next_back().unwrap_or(cmd).to_string();

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            request_id: Arc::new(Mutex::new(2)), // Start from 2 since we used 1 for init
            pending_requests,
            notification_tx,
            child: Arc::new(Mutex::new(child)),
            poll_endpoints: Arc::new(Mutex::new(Vec::new())),
            poll_shutdown_tx: Arc::new(Mutex::new(None)),
            server_name,
        })
    }

    /// Background reader loop - routes responses to pending requests, notifications to broadcast
    fn reader_loop(
        mut reader: BufReader<ChildStdout>,
        pending_requests: PendingRequests,
        notification_tx: broadcast::Sender<JsonRpcNotification>,
    ) {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    log_mcp("MCP server closed stdout");
                    break;
                }
                Ok(_) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }

                    // Log all incoming messages when debugging
                    if debug_mcp() {
                        eprintln!("[MCP] << {line}");
                    }

                    // Parse as generic JSON-RPC message
                    match serde_json::from_str::<JsonRpcMessage>(line) {
                        Ok(JsonRpcMessage::Response {
                            id, result, error, ..
                        }) => {
                            // Route response to pending request (sync or async)
                            if let Ok(mut pending) = pending_requests.lock() {
                                if let Some(sender) = pending.remove(&id) {
                                    let response = JsonRpcResponse {
                                        jsonrpc: "2.0".to_string(),
                                        id,
                                        result,
                                        error,
                                    };
                                    match sender {
                                        PendingRequest::Sync(tx) => {
                                            let _ = tx.send(response);
                                        }
                                        PendingRequest::Async(tx) => {
                                            let _ = tx.send(response);
                                        }
                                    }
                                } else {
                                    log_mcp(&format!("No pending request for id {id}"));
                                }
                            }
                        }
                        Ok(JsonRpcMessage::Notification { method, params, .. }) => {
                            // Broadcast notification to subscribers
                            eprintln!("[MCP] Notification received: {method}");
                            let notification = JsonRpcNotification { method, params };
                            let _ = notification_tx.send(notification);
                        }
                        Err(e) => {
                            log_mcp(&format!("Failed to parse message: {e} - {line}"));
                        }
                    }
                }
                Err(e) => {
                    log_mcp(&format!("Read error: {e}"));
                    break;
                }
            }
        }
    }

    pub fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        log_mcp("Requesting tools/list");
        let response = self.send_request("tools/list", Some(json!({})))?;

        if let Some(error) = &response.error {
            log_mcp(&format!("tools/list FAILED: {}", error.message));
            return Ok(vec![]);
        }

        if let Some(result) = response.result {
            // Check for tools in different possible locations
            if let Some(tools_value) = result.get("tools") {
                let tools: Vec<Tool> = serde_json::from_value(tools_value.clone())?;
                let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
                log_mcp(&format!(
                    "Discovered {} tools: {:?}",
                    tools.len(),
                    tool_names
                ));
                Ok(tools)
            } else if let Some(tools_value) = result
                .get("capabilities")
                .and_then(|c| c.get("tools"))
                .and_then(|t| t.get("available"))
            {
                let tools: Vec<Tool> = serde_json::from_value(tools_value.clone())?;
                let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
                log_mcp(&format!(
                    "Discovered {} tools: {:?}",
                    tools.len(),
                    tool_names
                ));
                Ok(tools)
            } else {
                log_mcp("WARNING: No tools found in response");
                log_mcp(&format!(
                    "Response structure: {}",
                    serde_json::to_string(&result)?
                ));
                Ok(vec![])
            }
        } else {
            log_mcp("WARNING: Empty result from tools/list");
            Ok(vec![])
        }
    }

    #[allow(clippy::missing_panics_doc)]
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
                .map(|d| d.as_secs())
                .unwrap_or(0),
            llm_origin: llm_origin.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            request_id: *self.request_id.lock().unwrap_or_else(|e| e.into_inner()),
        };

        log_mcp(&format!(
            "Tool call: {} args={}",
            tool_name,
            serde_json::to_string(&arguments).unwrap_or_default()
        ));

        // Log the tool call (always visible)
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
            log_mcp(&format!("Tool call FAILED: {}", error.message));
            return Err(format!("Tool execution error: {}", error.message).into());
        }

        // Extract the text content from the MCP response format
        if let Some(result) = response.result {
            log_mcp(&format!(
                "Tool response: {}",
                serde_json::to_string(&result).unwrap_or_default()
            ));

            // Check MCP-level isError (distinct from JSON-RPC errors).
            // Many MCP servers return isError: true with error details in content.
            let is_tool_error = result
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if let Some(content_array) = result.get("content").and_then(|c| c.as_array())
                && let Some(first_content) = content_array.first()
                && let Some(text) = first_content.get("text").and_then(|t| t.as_str())
            {
                if is_tool_error {
                    log_mcp(&format!("Tool returned isError: {text}"));
                    return Err(format!("Tool error: {text}").into());
                }
                return Ok(json!({ "result": text }));
            }
            Ok(result)
        } else {
            log_mcp("WARNING: Empty result from tool call");
            Ok(json!({}))
        }
    }

    /// Subscribe to notifications from this MCP server
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }

    /// Get the process ID of the MCP server subprocess
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().ok().map(|c| c.id())
    }

    fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, Box<dyn std::error::Error>> {
        // Get next request ID
        let request_id = {
            let mut id = self
                .request_id
                .lock()
                .map_err(|_| "Request ID lock poisoned")?;
            let current = *id;
            *id += 1;
            current
        };

        // Create sync channel for response (works in both sync and async contexts)
        let (tx, rx) = mpsc::channel();

        // Register pending request (sync mode)
        {
            let mut pending = self
                .pending_requests
                .lock()
                .map_err(|_| "Pending requests lock poisoned")?;
            pending.insert(request_id, PendingRequest::Sync(tx));
        }

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: request_id,
            method: method.to_string(),
            params,
        };

        // Send request
        {
            let mut stdin = self.stdin.lock().map_err(|_| "Stdin lock poisoned")?;
            writeln!(stdin, "{}", serde_json::to_string(&request)?)?;
            stdin.flush()?;
        }

        // Wait for response with timeout (background thread will send it)
        rx.recv_timeout(Duration::from_secs(30))
            .map_err(|e| format!("Response timeout or channel closed: {e}").into())
    }

    /// Async version of send_request for use in polling loop (non-blocking)
    pub async fn send_request_async(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Get next request ID
        let request_id = {
            let mut id = self
                .request_id
                .lock()
                .map_err(|_| "Request ID lock poisoned")?;
            let current = *id;
            *id += 1;
            current
        };

        // Create oneshot channel for async response
        let (tx, rx) = oneshot::channel();

        // Register pending request (async mode)
        {
            let mut pending = self
                .pending_requests
                .lock()
                .map_err(|_| "Pending requests lock poisoned")?;
            pending.insert(request_id, PendingRequest::Async(tx));
        }

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: request_id,
            method: method.to_string(),
            params,
        };

        // Send request
        {
            let mut stdin = self.stdin.lock().map_err(|_| "Stdin lock poisoned")?;
            writeln!(stdin, "{}", serde_json::to_string(&request)?)?;
            stdin.flush()?;
        }

        // Async await with timeout
        tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| "Request timeout")?
            .map_err(|_| "Channel closed".into())
    }

    /// Register a pollable endpoint
    pub fn register_pollable(&self, endpoint: PollableEndpoint) {
        if let Ok(mut endpoints) = self.poll_endpoints.lock() {
            endpoints.push(endpoint);
        }
    }

    /// Unregister a pollable endpoint by method name
    pub fn unregister_pollable(&self, method: &str) -> bool {
        if let Ok(mut endpoints) = self.poll_endpoints.lock() {
            let len_before = endpoints.len();
            endpoints.retain(|e| e.method != method);
            return endpoints.len() < len_before;
        }
        false
    }

    /// Get list of registered pollable endpoint methods
    pub fn list_pollables(&self) -> Vec<String> {
        self.poll_endpoints
            .lock()
            .map(|e| e.iter().map(|ep| ep.method.clone()).collect())
            .unwrap_or_default()
    }

    /// Start the polling loop with given configuration
    pub fn start_polling(self: &Arc<Self>, config: PollConfig) {
        // Check if already polling
        if self.poll_shutdown_tx.lock().is_ok_and(|g| g.is_some()) {
            log_mcp("Polling already started");
            return;
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Store shutdown sender
        if let Ok(mut guard) = self.poll_shutdown_tx.lock() {
            *guard = Some(shutdown_tx);
        }

        let client = Arc::clone(self);
        tokio::spawn(async move {
            polling_loop(client, config, shutdown_rx).await;
        });

        log_mcp("Polling loop started");
    }

    /// Stop the polling loop
    pub fn stop_polling(&self) {
        if let Ok(mut guard) = self.poll_shutdown_tx.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(true);
            log_mcp("Polling loop stopped");
        }
    }

    /// Get the server name (used in poll notifications)
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Set the server name (useful when the auto-detected name isn't ideal)
    pub fn set_server_name(&mut self, name: String) {
        self.server_name = name;
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
        // Stop polling first
        self.stop_polling();
        // Then kill the subprocess
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

/// Check if a JSON value represents an empty result
fn is_empty_result(result: Option<&Value>) -> bool {
    match result {
        None => true,
        Some(Value::Null) => true,
        Some(Value::Array(arr)) => arr.is_empty(),
        Some(Value::Object(obj)) => obj.is_empty(),
        Some(Value::String(s)) => s.is_empty(),
        _ => false,
    }
}

/// HTTP Streamable MCP client for connecting to remote MCP servers via HTTP POST.
/// Uses `reqwest::blocking` to avoid nested tokio runtime issues.
#[derive(Debug, Clone)]
pub struct HttpMcpClient {
    url: String,
    session_id: Arc<Mutex<Option<String>>>,
    request_id: Arc<Mutex<u64>>,
    client: Arc<reqwest::blocking::Client>,
    notification_tx: broadcast::Sender<JsonRpcNotification>,
    server_name: String,
}

impl HttpMcpClient {
    /// Create a new HTTP MCP client and perform the initialize handshake.
    #[allow(clippy::missing_panics_doc)]
    pub fn new(url: String) -> Result<Self, Box<dyn std::error::Error>> {
        log_mcp(&format!("HTTP MCP: connecting to {url}"));

        // Build blocking client in a dedicated thread to avoid tokio runtime conflicts
        let url_clone = url.clone();
        let (client, session_id) = std::thread::spawn(
            move || -> Result<_, Box<dyn std::error::Error + Send + Sync>> {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build()?;

                // Initialize handshake
                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": {
                            "name": "arkavo-agent",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                });

                let resp = client
                    .post(&url_clone)
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .map_err(|e| format!("HTTP MCP initialize failed: {e}"))?;

                let session_id = resp
                    .headers()
                    .get("mcp-session-id")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());

                if let Some(ref sid) = session_id {
                    log_mcp(&format!("HTTP MCP: session={sid}"));
                }

                let _body: serde_json::Value = resp
                    .json()
                    .map_err(|e| format!("HTTP MCP init parse: {e}"))?;

                log_mcp("HTTP MCP: initialized");
                Ok((client, session_id))
            },
        )
        .join()
        .expect("init thread panicked")
        .map_err(|e| -> Box<dyn std::error::Error> { e })?;

        let (notification_tx, _) = broadcast::channel(64);

        Ok(Self {
            url,
            session_id: Arc::new(Mutex::new(session_id)),
            request_id: Arc::new(Mutex::new(2)), // 1 was used for initialize
            client: Arc::new(client),
            notification_tx,
            server_name: String::new(),
        })
    }

    fn next_id(&self) -> u64 {
        let mut id = self.request_id.lock().unwrap();
        let current = *id;
        *id += 1;
        current
    }

    fn session_header(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|s| s.clone())
    }

    /// Re-initialize the MCP session (e.g. after session expiry or server restart).
    fn reinitialize(&self) -> Result<(), Box<dyn std::error::Error>> {
        log_mcp("HTTP MCP: re-initializing session");
        let url = self.url.clone();
        let client = self.client.clone();

        let new_session = std::thread::spawn(
            move || -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": {
                            "name": "arkavo-agent",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                });
                let resp = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .map_err(|e| format!("HTTP MCP reinit: {e}"))?;
                let sid = resp
                    .headers()
                    .get("mcp-session-id")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let _body: serde_json::Value = resp
                    .json()
                    .map_err(|e| format!("HTTP MCP reinit parse: {e}"))?;
                Ok(sid)
            },
        )
        .join()
        .expect("reinit thread panicked")
        .map_err(|e| -> Box<dyn std::error::Error> { e })?;

        if let Ok(mut sid) = self.session_id.lock() {
            *sid = new_session;
        }
        log_mcp("HTTP MCP: session re-initialized");
        Ok(())
    }

    /// Send a blocking JSON-RPC request in a dedicated thread (safe from async contexts).
    /// Automatically re-initializes the session on session/connection errors and retries once.
    fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, Box<dyn std::error::Error>> {
        match self.send_request_once(method, params.clone()) {
            Ok(result) => {
                // Check for JSON-RPC session errors
                if let Some(ref error) = result.error
                    && (error.message.contains("Session")
                        || error.message.contains("session")
                        || error.message.contains("initialize"))
                {
                    log_mcp(&format!(
                        "HTTP MCP: session error '{}', re-initializing",
                        error.message
                    ));
                    if self.reinitialize().is_ok() {
                        return self.send_request_once(method, params);
                    }
                }
                Ok(result)
            }
            Err(e) => {
                // Connection-level failure (server down, refused, timeout)
                let msg = e.to_string();
                if msg.contains("connect")
                    || msg.contains("refused")
                    || msg.contains("timeout")
                    || msg.contains("reset")
                    || msg.contains("broken pipe")
                    || msg.contains("HTTP MCP send")
                {
                    log_mcp(&format!(
                        "HTTP MCP: connection error '{msg}', re-initializing",
                    ));
                    if self.reinitialize().is_ok() {
                        return self.send_request_once(method, params);
                    }
                }
                Err(e)
            }
        }
    }

    fn send_request_once(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, Box<dyn std::error::Error>> {
        let id = self.next_id();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.unwrap_or_else(|| serde_json::json!({}))
        });

        let url = self.url.clone();
        let session = self.session_header();
        let client = self.client.clone();

        // Run blocking HTTP in a dedicated thread to avoid blocking the tokio runtime
        let result = std::thread::spawn(move || -> Result<JsonRpcResponse, String> {
            let mut req = client.post(&url).header("Content-Type", "application/json");

            if let Some(sid) = session {
                req = req.header("Mcp-Session-Id", sid);
            }

            let resp = req
                .json(&body)
                .send()
                .map_err(|e| format!("HTTP MCP send: {e}"))?;
            resp.json::<JsonRpcResponse>()
                .map_err(|e| format!("HTTP MCP parse: {e}"))
        })
        .join()
        .expect("request thread panicked")
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

        Ok(result)
    }

    pub fn list_tools(&self) -> Result<Vec<Tool>, Box<dyn std::error::Error>> {
        log_mcp("HTTP MCP: tools/list");
        let response = self.send_request("tools/list", Some(json!({})))?;

        if let Some(error) = &response.error {
            log_mcp(&format!("HTTP MCP tools/list FAILED: {}", error.message));
            return Ok(vec![]);
        }

        if let Some(result) = response.result {
            if let Some(tools_value) = result.get("tools") {
                let tools: Vec<Tool> = serde_json::from_value(tools_value.clone())?;
                log_mcp(&format!("HTTP MCP: {} tools discovered", tools.len()));
                Ok(tools)
            } else {
                log_mcp("HTTP MCP: no tools in response");
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
        log_mcp(&format!("HTTP MCP tool call: {tool_name}"));
        eprintln!(
            "[MCP Tool Call] HTTP | LLM: {llm_origin} | Tool: {tool_name} | Args: {}",
            serde_json::to_string(&arguments).unwrap_or_default()
        );

        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", Some(params))?;

        if let Some(error) = response.error {
            return Err(format!("Tool execution error: {}", error.message).into());
        }

        if let Some(result) = response.result {
            let is_tool_error = result
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if let Some(content_array) = result.get("content").and_then(|c| c.as_array())
                && let Some(first_content) = content_array.first()
                && let Some(text) = first_content.get("text").and_then(|t| t.as_str())
            {
                if is_tool_error {
                    log_mcp(&format!("HTTP MCP tool returned isError: {text}"));
                    return Err(format!("Tool error: {text}").into());
                }
                return Ok(json!({ "result": text }));
            }
            Ok(result)
        } else {
            Ok(json!({}))
        }
    }

    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }

    pub fn set_server_name(&mut self, name: String) {
        self.server_name = name;
    }
}

/// Polling loop that runs in a tokio::spawn task
async fn polling_loop(
    client: Arc<McpClient>,
    config: PollConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut backoff = AdaptiveBackoff::new(config.clone());
    let min_cycle = backoff.min_cycle_time();

    loop {
        let cycle_start = Instant::now();

        // Snapshot endpoints (release lock before awaiting)
        let endpoints: Vec<PollableEndpoint> = {
            match client.poll_endpoints.lock() {
                Ok(guard) => guard.clone(),
                Err(_) => break,
            }
        };

        for endpoint in endpoints {
            // Check shutdown before each poll
            if *shutdown_rx.borrow() {
                return;
            }

            let start = Instant::now();
            match client
                .send_request_async(&endpoint.method, endpoint.params.clone())
                .await
            {
                Ok(response) => {
                    let latency = start.elapsed();
                    let has_data = !is_empty_result(response.result.as_ref());

                    backoff.record(latency, has_data);

                    if has_data {
                        // Emit normalized poll notification
                        let poll_result = PollResultParams {
                            server: client.server_name.clone(),
                            endpoint: endpoint.method.clone(),
                            result: response.result.unwrap_or(Value::Null),
                            latency_ms: latency.as_millis() as u64,
                            is_poll: true,
                        };

                        let notif = JsonRpcNotification {
                            method: "mcp.poll.result".to_string(),
                            params: Some(serde_json::to_value(poll_result).unwrap_or(Value::Null)),
                        };
                        let _ = client.notification_tx.send(notif);
                    }
                }
                Err(e) => {
                    log_mcp(&format!("Poll error for {}: {e}", endpoint.method));
                    backoff.record_error();
                }
            }
        }

        // Enforce rate limit
        let elapsed = cycle_start.elapsed();
        if let Some(remaining) = min_cycle.checked_sub(elapsed) {
            tokio::time::sleep(remaining).await;
        }

        // Apply backoff interval
        let interval = backoff.next_interval();
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return;
                }
            }
        }
    }
}
