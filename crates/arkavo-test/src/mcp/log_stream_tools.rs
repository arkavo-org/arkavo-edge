use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as AsyncCommand;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub struct LogStreamKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
    active_streams: Arc<Mutex<std::collections::HashMap<String, JoinHandle<()>>>>,
}

impl LogStreamKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "log_stream".to_string(),
                description: "Stream logs from iOS apps running in the simulator. Captures console output, diagnostic logs, and system messages.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["start", "stop", "status", "read"],
                            "description": "Action to perform on log stream"
                        },
                        "process_name": {
                            "type": "string",
                            "description": "Process name to filter logs (e.g., 'ArkavoReference')"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "stream_id": {
                            "type": "string",
                            "description": "Stream ID for stop/status actions"
                        },
                        "predicate": {
                            "type": "string",
                            "description": "Custom predicate for filtering logs (advanced)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Number of recent log lines to read (for 'read' action)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            device_manager,
            active_streams: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }
    
    async fn start_log_stream(&self, device_id: &str, process_name: Option<&str>, predicate: Option<&str>) -> Result<Value> {
        // Build predicate
        let filter_predicate = if let Some(custom_predicate) = predicate {
            custom_predicate.to_string()
        } else if let Some(process) = process_name {
            format!("process == \"{}\"", process)
        } else {
            "eventType == \"logMessage\"".to_string()
        };
        
        // Generate stream ID
        let stream_id = format!("{}_{}", device_id, chrono::Utc::now().timestamp());
        
        // Create log output file
        let log_file_path = format!("/tmp/arkavo_logs_{}.txt", stream_id);
        
        // Start the log stream process
        let mut cmd = AsyncCommand::new("xcrun");
        cmd.args([
            "simctl",
            "spawn",
            device_id,
            "log",
            "stream",
            "--predicate",
            &filter_predicate,
            "--style",
            "json"
        ]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        
        let mut child = cmd.spawn()
            .map_err(|e| TestError::Mcp(format!("Failed to start log stream: {}", e)))?;
        
        let stdout = child.stdout.take()
            .ok_or_else(|| TestError::Mcp("Failed to capture stdout".to_string()))?;
        
        // Spawn task to read and save logs
        let log_file_path_clone = log_file_path.clone();
        let stream_id_clone = stream_id.clone();
        let handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let file = std::fs::OpenOptions::new()
                .create(true)
                
                .append(true)
                .open(&log_file_path_clone)
                .ok();
                
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(file) = &file {
                    use std::io::Write;
                    let mut file = file;
                    writeln!(file, "{}", line).ok();
                }
                
                // Parse JSON log entry if possible
                if let Ok(log_entry) = serde_json::from_str::<Value>(&line) {
                    if let Some(message) = log_entry.get("eventMessage").and_then(|m| m.as_str()) {
                        eprintln!("[LOG:{}] {}", stream_id_clone, message);
                    }
                }
            }
        });
        
        // Store the handle
        self.active_streams.lock().await.insert(stream_id.clone(), handle);
        
        Ok(json!({
            "success": true,
            "action": "start",
            "stream_id": stream_id,
            "device_id": device_id,
            "process_name": process_name,
            "predicate": filter_predicate,
            "log_file": log_file_path,
            "message": "Log stream started successfully"
        }))
    }
    
    async fn stop_log_stream(&self, stream_id: &str) -> Result<Value> {
        let mut streams = self.active_streams.lock().await;
        
        if let Some(handle) = streams.remove(stream_id) {
            handle.abort();
            
            Ok(json!({
                "success": true,
                "action": "stop",
                "stream_id": stream_id,
                "message": "Log stream stopped"
            }))
        } else {
            Ok(json!({
                "success": false,
                "action": "stop",
                "stream_id": stream_id,
                "error": "Stream not found"
            }))
        }
    }
    
    async fn get_stream_status(&self) -> Result<Value> {
        let streams = self.active_streams.lock().await;
        let active_streams: Vec<String> = streams.keys().cloned().collect();
        
        Ok(json!({
            "success": true,
            "action": "status",
            "active_streams": active_streams,
            "count": active_streams.len()
        }))
    }
    
    async fn read_recent_logs(&self, stream_id: Option<&str>, limit: Option<usize>) -> Result<Value> {
        let log_file_path = if let Some(id) = stream_id {
            format!("/tmp/arkavo_logs_{}.txt", id)
        } else {
            // Find most recent log file
            let entries = std::fs::read_dir("/tmp/")
                .map_err(|e| TestError::Mcp(format!("Failed to read log directory: {}", e)))?;
            
            let mut log_files: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name().to_string_lossy().starts_with("arkavo_logs_")
                })
                .collect();
                
            log_files.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            
            match log_files.last() {
                Some(file) => file.path().to_string_lossy().to_string(),
                None => return Ok(json!({
                    "success": false,
                    "error": "No log files found"
                }))
            }
        };
        
        // Read the log file
        let contents = std::fs::read_to_string(&log_file_path)
            .map_err(|e| TestError::Mcp(format!("Failed to read log file: {}", e)))?;
        
        let lines: Vec<&str> = contents.lines().collect();
        let limit = limit.unwrap_or(100);
        let start = lines.len().saturating_sub(limit);
        let recent_lines: Vec<String> = lines[start..].iter().map(|s| s.to_string()).collect();
        
        // Parse JSON logs if possible
        let mut parsed_logs = Vec::new();
        for line in &recent_lines {
            if let Ok(log_entry) = serde_json::from_str::<Value>(line) {
                parsed_logs.push(log_entry);
            } else {
                parsed_logs.push(json!({"raw": line}));
            }
        }
        
        Ok(json!({
            "success": true,
            "action": "read",
            "log_file": log_file_path,
            "line_count": recent_lines.len(),
            "logs": parsed_logs
        }))
    }
}

#[async_trait]
impl Tool for LogStreamKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;
        
        match action {
            "start" => {
                // Get device ID
                let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
                    id.to_string()
                } else {
                    match self.device_manager.get_active_device() {
                        Some(device) => device.id,
                        None => {
                            self.device_manager.refresh_devices().ok();
                            match self.device_manager.get_booted_devices().first() {
                                Some(device) => device.id.clone(),
                                None => {
                                    return Ok(json!({
                                        "error": {
                                            "code": "NO_BOOTED_DEVICE",
                                            "message": "No booted iOS device found"
                                        }
                                    }));
                                }
                            }
                        }
                    }
                };
                
                let process_name = params.get("process_name").and_then(|v| v.as_str());
                let predicate = params.get("predicate").and_then(|v| v.as_str());
                
                self.start_log_stream(&device_id, process_name, predicate).await
            }
            
            "stop" => {
                let stream_id = params
                    .get("stream_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing stream_id for stop action".to_string()))?;
                
                self.stop_log_stream(stream_id).await
            }
            
            "status" => {
                self.get_stream_status().await
            }
            
            "read" => {
                let stream_id = params.get("stream_id").and_then(|v| v.as_str());
                let limit = params.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
                
                self.read_recent_logs(stream_id, limit).await
            }
            
            _ => Err(TestError::Mcp(format!("Unknown action: {}", action)))
        }
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct AppDiagnosticExporter {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl AppDiagnosticExporter {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "app_diagnostic_export".to_string(),
                description: "Export diagnostic data from ArkavoReference app using deep links".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Bundle ID of the app (default: com.arkavo.reference)"
                        }
                    }
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for AppDiagnosticExporter {
    async fn execute(&self, params: Value) -> Result<Value> {
        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    self.device_manager.refresh_devices().ok();
                    match self.device_manager.get_booted_devices().first() {
                        Some(device) => device.id.clone(),
                        None => {
                            return Ok(json!({
                                "error": {
                                    "code": "NO_BOOTED_DEVICE",
                                    "message": "No booted iOS device found"
                                }
                            }));
                        }
                    }
                }
            }
        };
        
        let bundle_id = params
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .unwrap_or("com.arkavo.reference");
        
        // First ensure the app is running
        let launch_output = Command::new("xcrun")
            .args(["simctl", "launch", &device_id, bundle_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to launch app: {}", e)))?;
        
        if !launch_output.status.success() {
            return Ok(json!({
                "success": false,
                "error": {
                    "message": "Failed to launch app",
                    "details": String::from_utf8_lossy(&launch_output.stderr).trim().to_string()
                }
            }));
        }
        
        // Wait for app to launch
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        
        // Open the export deep link
        let export_url = "arkavo-reference://diagnostic/export";
        let output = Command::new("xcrun")
            .args(["simctl", "openurl", &device_id, export_url])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to open diagnostic export URL: {}", e)))?;
        
        if !output.status.success() {
            return Ok(json!({
                "success": false,
                "error": {
                    "message": "Failed to trigger diagnostic export",
                    "details": String::from_utf8_lossy(&output.stderr).trim().to_string()
                }
            }));
        }
        
        // The app logs the exported data to console, so we need to capture it
        // Start a temporary log stream to capture the export
        let predicate = format!("process == \"{}\"", "ArkavoReference");
        let mut log_cmd = Command::new("xcrun");
        log_cmd.args([
            "simctl",
            "spawn",
            &device_id,
            "log",
            "stream",
            "--predicate",
            &predicate,
            "--style",
            "compact"
        ]);
        log_cmd.stdout(Stdio::piped());
        
        // Wait a bit for the export to complete
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        // Kill the log process
        let _ = Command::new("pkill")
            .args(["-f", "log stream"])
            .output();
        
        Ok(json!({
            "success": true,
            "message": "Diagnostic export triggered",
            "device_id": device_id,
            "bundle_id": bundle_id,
            "note": "Check the log stream for exported diagnostic data",
            "hint": "Use log_stream tool with process_name='ArkavoReference' to capture diagnostic output"
        }))
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}