use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::event_mapper::EventMapper;
use crate::jsonrpc::{JsonRpcClient, JsonRpcMessage, JsonRpcRequest, JsonRpcResponse};
use crate::{ClaudeCodeError, Result};

type ToolInterceptor =
    Box<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> + Send + Sync>;

/// Bridge to Node.js subprocess running Claude Code SDK
pub struct NodeBridge {
    process: Arc<Mutex<Option<Child>>>,
    stdin_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    rpc_client: Arc<JsonRpcClient>,
    request_tx: mpsc::Sender<(JsonRpcRequest, oneshot::Sender<JsonRpcResponse>)>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    event_mapper: Arc<EventMapper>,
    tool_interceptor: Arc<RwLock<Option<ToolInterceptor>>>,
    anthropic_model: String,
    anthropic_base_url: Option<String>,
    anthropic_auth_token: Option<String>,
    anthropic_small_fast_model: Option<String>,
    node_path: Option<PathBuf>,
}

impl NodeBridge {
    /// Create a new Node.js bridge
    pub async fn new(
        config: &crate::config::ClaudeCodeConfig,
        event_mapper: Arc<EventMapper>,
    ) -> Result<Self> {
        let rpc_client = Arc::new(JsonRpcClient::new());
        let (request_tx, _request_rx) = mpsc::channel(100);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();

        Ok(Self {
            process: Arc::new(Mutex::new(None)),
            stdin_tx: Arc::new(Mutex::new(None)),
            rpc_client,
            request_tx,
            shutdown_tx: Some(shutdown_tx),
            event_mapper,
            tool_interceptor: Arc::new(RwLock::new(None)),
            anthropic_model: config.anthropic_model.clone(),
            anthropic_base_url: config.anthropic_base_url.clone(),
            anthropic_auth_token: config.anthropic_auth_token.clone(),
            anthropic_small_fast_model: config.anthropic_small_fast_model.clone(),
            node_path: config.node_path.clone(),
        })
    }

    /// Initialize the Node.js subprocess
    pub async fn initialize(&self) -> Result<()> {
        info!("Starting Node.js subprocess for Claude Code SDK");

        // Check if Node.js is available
        let node_cmd = self
            .node_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "node".to_string());

        // Test Node.js availability
        let node_test = tokio::process::Command::new(&node_cmd)
            .arg("--version")
            .output()
            .await;

        match node_test {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                info!("Node.js available: {}", version.trim());
            }
            _ => {
                return Err(ClaudeCodeError::NodeProcess(
                    "Node.js not found. Please install Node.js >= 18.0.0 and the Claude Code SDK:\n\
                     1. Install Node.js from https://nodejs.org/\n\
                     2. Run: npm install -g @anthropic-ai/claude-code\n\
                     3. Set ANTHROPIC_API_KEY environment variable".to_string()
                ));
            }
        }

        // Path to our Node.js bridge script - check multiple possible locations
        let possible_paths = vec![
            // Installed location (Homebrew, system install)
            PathBuf::from("/usr/local/share/arkavo/claude-code-bridge.js"),
            PathBuf::from("/opt/homebrew/share/arkavo/claude-code-bridge.js"),
            // Development location
            std::env::current_dir()
                .unwrap_or_default()
                .join("crates/arkavo-claude-code/node/claude-code-bridge.js"),
            // Relative to binary
            std::env::current_exe()
                .unwrap_or_default()
                .parent()
                .map(|p| p.join("../share/arkavo/claude-code-bridge.js"))
                .unwrap_or_default(),
        ];

        let script_path = possible_paths
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| ClaudeCodeError::NodeProcess(
                "Claude Code bridge script not found. Please ensure Arkavo is properly installed.".to_string()
            ))?;

        // Start Node.js process with environment variables
        let mut cmd = Command::new(&node_cmd);
        cmd.arg(script_path)
            .env("ANTHROPIC_MODEL", &self.anthropic_model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set API key or auth token
        if let Some(auth_token) = &self.anthropic_auth_token {
            cmd.env("ANTHROPIC_AUTH_TOKEN", auth_token);
        } else {
            cmd.env(
                "ANTHROPIC_API_KEY",
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            );
        }

        // Set optional environment variables
        if let Some(base_url) = &self.anthropic_base_url {
            cmd.env("ANTHROPIC_BASE_URL", base_url);
        }

        if let Some(small_model) = &self.anthropic_small_fast_model {
            cmd.env("ANTHROPIC_SMALL_FAST_MODEL", small_model);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| ClaudeCodeError::NodeProcess(format!("Failed to spawn Node.js: {}", e)))?;

        // Get handles to stdin/stdout
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClaudeCodeError::NodeProcess("Failed to get stdin".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClaudeCodeError::NodeProcess("Failed to get stdout".to_string()))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ClaudeCodeError::NodeProcess("Failed to get stderr".to_string()))?;

        // Set up stdin writer
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(100);
        let mut stdin_writer = stdin;

        tokio::spawn(async move {
            while let Some(msg) = stdin_rx.recv().await {
                if let Err(e) = stdin_writer.write_all(msg.as_bytes()).await {
                    tracing::error!("Failed to write to stdin: {}", e);
                    break;
                }
            }
        });

        *self.stdin_tx.lock().await = Some(stdin_tx);

        // Start message handler tasks
        self.start_message_handler(stdout, stderr).await;

        // Store the process handle
        let mut process_guard = self.process.lock().await;
        *process_guard = Some(child);

        // Send initialization request
        let init_request = self.rpc_client.create_request(
            "initialize".to_string(),
            Some(serde_json::json!({
                "model": self.anthropic_model,
            })),
        );

        let response = self.send_request(init_request).await?;

        if let Some(error) = response.error {
            return Err(ClaudeCodeError::NodeProcess(format!(
                "Initialization failed: {}",
                error.message
            )));
        }

        info!("Node.js subprocess initialized successfully");
        Ok(())
    }

    /// Start message handler tasks for stdout/stderr communication
    async fn start_message_handler(
        &self,
        stdout: tokio::process::ChildStdout,
        stderr: tokio::process::ChildStderr,
    ) {
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        // Handle stderr (log output from Node.js)
        tokio::spawn(async move {
            while let Ok(Some(line)) = stderr_reader.next_line().await {
                debug!("Node.js stderr: {}", line);
            }
        });

        // Handle stdout (JSON-RPC messages)
        let event_mapper = self.event_mapper.clone();
        let tool_interceptor = self.tool_interceptor.clone();

        tokio::spawn(async move {
            let mut pending_requests: std::collections::HashMap<
                String,
                oneshot::Sender<JsonRpcResponse>,
            > = std::collections::HashMap::new();

            while let Ok(Some(line)) = stdout_reader.next_line().await {
                // Parse JSON-RPC message
                match serde_json::from_str::<JsonRpcMessage>(&line) {
                    Ok(JsonRpcMessage::Response(response)) => {
                        // Handle response to our request
                        if let Some(id) = &response.id {
                            let id_str = match id {
                                crate::jsonrpc::JsonRpcId::Number(n) => n.to_string(),
                                crate::jsonrpc::JsonRpcId::String(s) => s.clone(),
                            };

                            if let Some(tx) = pending_requests.remove(&id_str) {
                                let _ = tx.send(response);
                            }
                        }
                    }
                    Ok(JsonRpcMessage::Notification(notification)) => {
                        // Handle notifications from Node.js
                        match notification.method.as_str() {
                            "event" => {
                                // Map and emit event
                                if let Some(params) = notification.params {
                                    event_mapper.handle_sdk_event(params).await;
                                }
                            }
                            "tool_request" => {
                                // Intercept tool request for policy check
                                if let Some(params) = notification.params {
                                    let interceptor_guard = tool_interceptor.read().await;
                                    if let Some(interceptor) = interceptor_guard.as_ref() {
                                        let _result = interceptor(params.clone()).await;
                                        // Send approval/denial back to Node.js
                                        // TODO: Implement response mechanism
                                    }
                                }
                            }
                            _ => {
                                debug!("Unhandled notification: {}", notification.method);
                            }
                        }
                    }
                    Ok(JsonRpcMessage::Request(request)) => {
                        // Handle requests from Node.js (shouldn't happen in our design)
                        warn!("Unexpected request from Node.js: {}", request.method);
                    }
                    Err(e) => {
                        error!("Failed to parse JSON-RPC message: {} - Line: {}", e, line);
                    }
                }
            }
        });
    }

    /// Send a request to Node.js and wait for response
    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let (response_tx, response_rx) = oneshot::channel();

        self.request_tx
            .send((request.clone(), response_tx))
            .await
            .map_err(|e| ClaudeCodeError::NodeProcess(format!("Failed to send request: {}", e)))?;

        // Write request to stdin via channel
        let stdin_tx = self.stdin_tx.lock().await;
        if let Some(tx) = stdin_tx.as_ref() {
            let request_str = serde_json::to_string(&request).map_err(ClaudeCodeError::Json)?;

            tx.send(format!("{}\n", request_str)).await.map_err(|e| {
                ClaudeCodeError::NodeProcess(format!("Failed to send to stdin: {}", e))
            })?;
        } else {
            return Err(ClaudeCodeError::NodeProcess(
                "Stdin channel not initialized".to_string(),
            ));
        }

        // Wait for response
        response_rx
            .await
            .map_err(|e| ClaudeCodeError::NodeProcess(format!("Response channel closed: {}", e)))
    }

    /// Open a new Claude Code session
    pub async fn open_session(&self, context: Option<Value>) -> Result<String> {
        let request = self.rpc_client.create_request(
            "openSession".to_string(),
            Some(serde_json::json!({
                "context": context,
            })),
        );

        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ClaudeCodeError::Sdk(error.message));
        }

        response
            .result
            .and_then(|r| {
                r.get("sessionId")
                    .and_then(|s| s.as_str())
                    .map(String::from)
            })
            .ok_or_else(|| ClaudeCodeError::Sdk("Missing sessionId in response".to_string()))
    }

    /// Run a task in the session
    pub async fn stream_run(&self, prompt: String, session_id: String) -> Result<()> {
        let request = self.rpc_client.create_request(
            "streamRun".to_string(),
            Some(serde_json::json!({
                "sessionId": session_id,
                "prompt": prompt,
            })),
        );

        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ClaudeCodeError::Sdk(error.message));
        }

        Ok(())
    }

    /// Close a session
    pub async fn close_session(&self, session_id: String) -> Result<()> {
        let request = self.rpc_client.create_request(
            "closeSession".to_string(),
            Some(serde_json::json!({
                "sessionId": session_id,
            })),
        );

        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ClaudeCodeError::Sdk(error.message));
        }

        Ok(())
    }

    /// Set tool interceptor for policy enforcement
    pub fn set_tool_interceptor<F>(&self, interceptor: F)
    where
        F: Fn(Value) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> + Send + Sync + 'static,
    {
        let mut guard = self.tool_interceptor.blocking_write();
        *guard = Some(Box::new(interceptor));
    }

    /// Shutdown the Node.js subprocess
    pub async fn shutdown(self) -> Result<()> {
        info!("Shutting down Node.js subprocess");

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx {
            let _ = tx.send(());
        }

        // Kill the process
        let mut process_guard = self.process.lock().await;
        if let Some(mut child) = process_guard.take() {
            child.kill().await.map_err(|e| {
                ClaudeCodeError::NodeProcess(format!("Failed to kill process: {}", e))
            })?;
        }

        Ok(())
    }
}
