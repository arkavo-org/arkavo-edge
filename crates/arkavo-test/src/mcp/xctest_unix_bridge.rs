use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandType {
    Tap,
    Swipe,
    TypeText,
    Scroll,
    LongPress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    #[serde(rename = "type")]
    pub command_type: CommandType,
    pub parameters: CommandParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandParameters {
    // Tap parameters
    pub target_type: Option<TargetType>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub text: Option<String>,
    pub accessibility_id: Option<String>,
    pub timeout: Option<f64>,

    // Swipe parameters
    pub x1: Option<f64>,
    pub y1: Option<f64>,
    pub x2: Option<f64>,
    pub y2: Option<f64>,
    pub duration: Option<f64>,

    // Type text parameters
    pub text_to_type: Option<String>,
    pub clear_first: Option<bool>,

    // Scroll parameters
    pub direction: Option<String>,
    pub distance: Option<f64>,

    // Long press parameters
    pub press_duration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetType {
    Coordinate,
    Text,
    AccessibilityId,
}

// Backwards compatibility
pub type TapCommand = Command;

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
    
    /// Create with a specific socket path
    pub fn with_socket_path(socket_path: PathBuf) -> Self {
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

        // Set socket permissions to be accessible only by owner
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&self.socket_path)?.permissions();
            perms.set_mode(0o600); // rw------- (owner read/write only)
            std::fs::set_permissions(&self.socket_path, perms)?;
        }

        Ok(())
    }
    
    /// Wait for a client to connect
    pub async fn wait_for_connection(&self) -> Result<()> {
        // Poll until we have a client connection
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(30);
        
        while start.elapsed() < timeout {
            if self.is_connected() {
                return Ok(());
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        
        Err(TestError::Mcp("No client connected within timeout".to_string()))
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

    /// Send a tap command and wait for response (backwards compatibility)
    pub async fn send_tap_command(&self, command: TapCommand) -> Result<CommandResponse> {
        self.send_command(command).await
    }

    /// Send a command and wait for response
    pub async fn send_command(&self, command: Command) -> Result<CommandResponse> {
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
    pub fn create_coordinate_tap(x: f64, y: f64) -> Command {
        Command {
            id: Uuid::new_v4().to_string(),
            command_type: CommandType::Tap,
            parameters: CommandParameters {
                target_type: Some(TargetType::Coordinate),
                x: Some(x),
                y: Some(y),
                text: None,
                accessibility_id: None,
                timeout: None,
                x1: None,
                y1: None,
                x2: None,
                y2: None,
                duration: None,
                text_to_type: None,
                clear_first: None,
                direction: None,
                distance: None,
                press_duration: None,
            },
        }
    }

    /// Create a tap command for text
    pub fn create_text_tap(text: String, timeout: Option<f64>) -> Command {
        Command {
            id: Uuid::new_v4().to_string(),
            command_type: CommandType::Tap,
            parameters: CommandParameters {
                target_type: Some(TargetType::Text),
                x: None,
                y: None,
                text: Some(text),
                accessibility_id: None,
                timeout,
                x1: None,
                y1: None,
                x2: None,
                y2: None,
                duration: None,
                text_to_type: None,
                clear_first: None,
                direction: None,
                distance: None,
                press_duration: None,
            },
        }
    }

    /// Create a tap command for accessibility ID
    pub fn create_accessibility_tap(accessibility_id: String, timeout: Option<f64>) -> Command {
        Command {
            id: Uuid::new_v4().to_string(),
            command_type: CommandType::Tap,
            parameters: CommandParameters {
                target_type: Some(TargetType::AccessibilityId),
                x: None,
                y: None,
                text: None,
                accessibility_id: Some(accessibility_id),
                timeout,
                x1: None,
                y1: None,
                x2: None,
                y2: None,
                duration: None,
                text_to_type: None,
                clear_first: None,
                direction: None,
                distance: None,
                press_duration: None,
            },
        }
    }

    /// Create a swipe command
    pub fn create_swipe(x1: f64, y1: f64, x2: f64, y2: f64, duration: Option<f64>) -> Command {
        Command {
            id: Uuid::new_v4().to_string(),
            command_type: CommandType::Swipe,
            parameters: CommandParameters {
                target_type: None,
                x: None,
                y: None,
                text: None,
                accessibility_id: None,
                timeout: None,
                x1: Some(x1),
                y1: Some(y1),
                x2: Some(x2),
                y2: Some(y2),
                duration: duration.or(Some(0.5)),
                text_to_type: None,
                clear_first: None,
                direction: None,
                distance: None,
                press_duration: None,
            },
        }
    }

    /// Create a type text command
    pub fn create_type_text(text: String, clear_first: bool) -> Command {
        Command {
            id: Uuid::new_v4().to_string(),
            command_type: CommandType::TypeText,
            parameters: CommandParameters {
                target_type: None,
                x: None,
                y: None,
                text: None,
                accessibility_id: None,
                timeout: None,
                x1: None,
                y1: None,
                x2: None,
                y2: None,
                duration: None,
                text_to_type: Some(text),
                clear_first: Some(clear_first),
                direction: None,
                distance: None,
                press_duration: None,
            },
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
        assert_eq!(coord_tap.parameters.x, Some(100.0));
        assert_eq!(coord_tap.parameters.y, Some(200.0));

        let text_tap = XCTestUnixBridge::create_text_tap("Login".to_string(), Some(5.0));
        assert_eq!(text_tap.parameters.text, Some("Login".to_string()));
        assert_eq!(text_tap.parameters.timeout, Some(5.0));

        let acc_tap = XCTestUnixBridge::create_accessibility_tap("login_button".to_string(), None);
        assert_eq!(
            acc_tap.parameters.accessibility_id,
            Some("login_button".to_string())
        );
    }
}
