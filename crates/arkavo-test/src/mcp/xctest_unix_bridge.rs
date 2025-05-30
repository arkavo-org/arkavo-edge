use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapCommand {
    pub id: String,
    #[serde(rename = "targetType")]
    pub target_type: TargetType,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub text: Option<String>,
    #[serde(rename = "accessibilityId")]
    pub accessibility_id: Option<String>,
    pub timeout: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetType {
    Coordinate,
    Text,
    AccessibilityId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub id: String,
    pub success: bool,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
}

pub struct XCTestUnixBridge {
    socket_path: PathBuf,
    response_handlers: Arc<Mutex<HashMap<String, oneshot::Sender<CommandResponse>>>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    client_stream: Option<Arc<Mutex<UnixStream>>>,
}

impl Default for XCTestUnixBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl XCTestUnixBridge {
    pub fn new() -> Self {
        // Use a unique socket path in tmp directory
        let socket_path =
            std::env::temp_dir().join(format!("arkavo-xctest-{}.sock", std::process::id()));

        Self {
            socket_path,
            response_handlers: Arc::new(Mutex::new(HashMap::new())),
            server_handle: None,
            client_stream: None,
        }
    }

    /// Check if the bridge is connected to the XCTest runner
    pub fn is_connected(&self) -> bool {
        self.client_stream.is_some()
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Start the Unix socket server
    pub async fn start(&mut self) -> Result<()> {
        // Remove existing socket file if it exists
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)
                .map_err(|e| TestError::Mcp(format!("Failed to remove existing socket: {}", e)))?;
        }

        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|e| TestError::Mcp(format!("Failed to bind Unix socket: {}", e)))?;

        eprintln!(
            "XCTest Unix bridge listening on: {}",
            self.socket_path.display()
        );

        let response_handlers = self.response_handlers.clone();
        let _socket_path = self.socket_path.clone();

        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let handlers = response_handlers.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_client(stream, handlers).await {
                                eprintln!("Client handler error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to accept connection: {}", e);
                    }
                }
            }
        });

        self.server_handle = Some(handle);

        // Set socket permissions to be accessible
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&self.socket_path)?.permissions();
            perms.set_mode(0o666); // rw-rw-rw-
            std::fs::set_permissions(&self.socket_path, perms)?;
        }

        Ok(())
    }

    /// Connect to the XCTest runner (for sending commands)
    pub async fn connect_to_runner(&mut self) -> Result<()> {
        // Wait a bit for the Swift side to start listening
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| TestError::Mcp(format!("Failed to connect to XCTest runner: {}", e)))?;

        self.client_stream = Some(Arc::new(Mutex::new(stream)));
        Ok(())
    }

    /// Send a tap command and wait for response
    pub async fn send_tap_command(&self, command: TapCommand) -> Result<CommandResponse> {
        let stream = self
            .client_stream
            .as_ref()
            .ok_or_else(|| TestError::Mcp("Not connected to XCTest runner".to_string()))?
            .clone();

        let (tx, rx) = oneshot::channel();

        // Register response handler
        {
            let mut handlers = self.response_handlers.lock().await;
            handlers.insert(command.id.clone(), tx);
        }

        // Send command
        let command_json = serde_json::to_string(&command)
            .map_err(|e| TestError::Mcp(format!("Failed to serialize command: {}", e)))?;

        {
            let mut stream = stream.lock().await;
            stream
                .write_all(command_json.as_bytes())
                .await
                .map_err(|e| TestError::Mcp(format!("Failed to send command: {}", e)))?;
            stream
                .write_all(b"\n")
                .await
                .map_err(|e| TestError::Mcp(format!("Failed to send newline: {}", e)))?;
            stream
                .flush()
                .await
                .map_err(|e| TestError::Mcp(format!("Failed to flush stream: {}", e)))?;
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(TestError::Mcp("Response channel closed".to_string())),
            Err(_) => {
                // Remove handler on timeout
                let mut handlers = self.response_handlers.lock().await;
                handlers.remove(&command.id);

                Err(TestError::Mcp("Command timed out".to_string()))
            }
        }
    }

    /// Create a tap command for coordinates
    pub fn create_coordinate_tap(x: f64, y: f64) -> TapCommand {
        TapCommand {
            id: Uuid::new_v4().to_string(),
            target_type: TargetType::Coordinate,
            x: Some(x),
            y: Some(y),
            text: None,
            accessibility_id: None,
            timeout: None,
        }
    }

    /// Create a tap command for text
    pub fn create_text_tap(text: String, timeout: Option<f64>) -> TapCommand {
        TapCommand {
            id: Uuid::new_v4().to_string(),
            target_type: TargetType::Text,
            x: None,
            y: None,
            text: Some(text),
            accessibility_id: None,
            timeout,
        }
    }

    /// Create a tap command for accessibility ID
    pub fn create_accessibility_tap(accessibility_id: String, timeout: Option<f64>) -> TapCommand {
        TapCommand {
            id: Uuid::new_v4().to_string(),
            target_type: TargetType::AccessibilityId,
            x: None,
            y: None,
            text: None,
            accessibility_id: Some(accessibility_id),
            timeout,
        }
    }
}

impl Drop for XCTestUnixBridge {
    fn drop(&mut self) {
        // Clean up socket file
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

/// Handle incoming client connections
async fn handle_client(
    stream: UnixStream,
    response_handlers: Arc<Mutex<HashMap<String, oneshot::Sender<CommandResponse>>>>,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                // Parse response
                if let Ok(response) = serde_json::from_str::<CommandResponse>(&line) {
                    // Find and notify the waiting handler
                    let handler = {
                        let mut handlers = response_handlers.lock().await;
                        handlers.remove(&response.id)
                    };

                    if let Some(tx) = handler {
                        let _ = tx.send(response);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error reading from client: {}", e);
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_unix_bridge_creation() {
        let bridge = XCTestUnixBridge::new();
        assert!(
            bridge
                .socket_path
                .to_string_lossy()
                .contains("arkavo-xctest")
        );
    }

    #[test]
    fn test_command_creation() {
        let coord_tap = XCTestUnixBridge::create_coordinate_tap(100.0, 200.0);
        assert_eq!(coord_tap.x, Some(100.0));
        assert_eq!(coord_tap.y, Some(200.0));

        let text_tap = XCTestUnixBridge::create_text_tap("Login".to_string(), Some(5.0));
        assert_eq!(text_tap.text, Some("Login".to_string()));
        assert_eq!(text_tap.timeout, Some(5.0));

        let acc_tap = XCTestUnixBridge::create_accessibility_tap("login_button".to_string(), None);
        assert_eq!(acc_tap.accessibility_id, Some("login_button".to_string()));
    }
}
