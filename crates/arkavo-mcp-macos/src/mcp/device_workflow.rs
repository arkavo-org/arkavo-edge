use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

// Global store for device log capture sessions
static DEVICE_LOG_SESSIONS: std::sync::LazyLock<Arc<Mutex<HashMap<String, DeviceLogSession>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Physical device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalDevice {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub os_version: String,
    pub connection_type: String,
    pub is_available: bool,
}

/// Device log capture session
struct DeviceLogSession {
    child: Child,
    output_buffer: Arc<std::sync::Mutex<Vec<String>>>,
    device_id: String,
    start_time: Instant,
}

/// Execute devicectl command (Xcode 15+)
async fn run_devicectl(args: &[&str]) -> Result<String> {
    let output = Command::new("xcrun")
        .arg("devicectl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| TestError::Execution(format!("Failed to run devicectl: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TestError::Execution(format!(
            "devicectl failed: {}",
            stderr
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Execute xcodebuild command with timeout
async fn run_xcodebuild(args: &[&str], timeout_secs: u64) -> Result<(bool, String, String, u64)> {
    let start = Instant::now();

    let mut cmd = Command::new("xcodebuild");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let timeout_duration = Duration::from_secs(timeout_secs.min(600));

    let child = cmd
        .spawn()
        .map_err(|e| TestError::Execution(format!("Failed to run xcodebuild: {}", e)))?;

    let output = tokio::time::timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| {
            TestError::Execution(format!("Build timed out after {} seconds", timeout_secs))
        })?
        .map_err(|e| TestError::Execution(format!("xcodebuild failed: {}", e)))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok((output.status.success(), stdout, stderr, duration_ms))
}

// ============================================================================
// DeviceListTool - List connected physical devices
// ============================================================================

pub struct DeviceListTool {
    schema: ToolSchema,
}

impl DeviceListTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_list".to_string(),
                aliases: Some(vec![
                    "list_devices".to_string(),
                    "physical_devices".to_string(),
                ]),
                description: "List connected physical Apple devices (iPhone, iPad, Apple Watch)."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "available_only": {
                            "type": "boolean",
                            "description": "Only show available/connected devices (default: true)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for DeviceListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceListTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let available_only = params
            .get("available_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let output = run_devicectl(&["list", "devices", "--json-output", "/dev/stdout"]).await?;

        let json_output: Value = serde_json::from_str(&output).unwrap_or(json!({}));

        let devices: Vec<PhysicalDevice> = json_output
            .get("result")
            .and_then(|r| r.get("devices"))
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| {
                        let udid = d
                            .get("hardwareProperties")
                            .and_then(|h| h.get("udid"))
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string();

                        if udid.is_empty() {
                            return None;
                        }

                        let name = d
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("Unknown")
                            .to_string();
                        let model = d
                            .get("hardwareProperties")
                            .and_then(|h| h.get("marketingName"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("Unknown")
                            .to_string();
                        let os_version = d
                            .get("deviceProperties")
                            .and_then(|dp| dp.get("osVersionNumber"))
                            .and_then(|o| o.as_str())
                            .unwrap_or("Unknown")
                            .to_string();
                        let connection_type = d
                            .get("connectionProperties")
                            .and_then(|c| c.get("transportType"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        Some(PhysicalDevice {
                            udid,
                            name,
                            model,
                            os_version,
                            connection_type,
                            is_available: true,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let filtered: Vec<_> = if available_only {
            devices.into_iter().filter(|d| d.is_available).collect()
        } else {
            devices
        };

        Ok(json!({
            "success": true,
            "count": filtered.len(),
            "devices": filtered
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceBuildTool - Build app for physical device
// ============================================================================

pub struct DeviceBuildTool {
    schema: ToolSchema,
}

impl DeviceBuildTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_build".to_string(),
                aliases: Some(vec!["build_device".to_string()]),
                description: "Build an Xcode project for a physical iOS device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace"
                        },
                        "scheme": {
                            "type": "string",
                            "description": "Build scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["Debug", "Release"],
                            "description": "Build configuration (default: Debug)"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Target device UDID (optional, for signing)"
                        },
                        "derived_data_path": {
                            "type": "string",
                            "description": "Custom derived data path"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Build timeout in seconds (default: 300)"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for DeviceBuildTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceBuildTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        let mut args = vec!["build", "-scheme", scheme, "-sdk", "iphoneos"];

        let workspace;
        let project;
        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            workspace = ws.to_string();
            args.push("-workspace");
            args.push(&workspace);
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            project = proj.to_string();
            args.push("-project");
            args.push(&project);
        }

        let config;
        if let Some(c) = params.get("configuration").and_then(|v| v.as_str()) {
            config = c.to_string();
            args.push("-configuration");
            args.push(&config);
        }

        let destination;
        if let Some(device_id) = params.get("device_id").and_then(|v| v.as_str()) {
            destination = format!("platform=iOS,id={}", device_id);
            args.push("-destination");
            args.push(&destination);
        }

        let derived_data;
        if let Some(dd) = params.get("derived_data_path").and_then(|v| v.as_str()) {
            derived_data = dd.to_string();
            args.push("-derivedDataPath");
            args.push(&derived_data);
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        Ok(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceInstallTool - Install app on device
// ============================================================================

pub struct DeviceInstallTool {
    schema: ToolSchema,
}

impl DeviceInstallTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_install".to_string(),
                aliases: Some(vec!["install_app_device".to_string()]),
                description: "Install an app (.app or .ipa) on a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "app_path": {
                            "type": "string",
                            "description": "Path to .app bundle or .ipa file"
                        }
                    },
                    "required": ["device_id", "app_path"]
                }),
            },
        }
    }
}

impl Default for DeviceInstallTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceInstallTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'device_id' is required".to_string()))?;

        let app_path = params
            .get("app_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'app_path' is required".to_string()))?;

        let output =
            run_devicectl(&["device", "install", "app", "--device", device_id, app_path]).await?;

        Ok(json!({
            "success": true,
            "message": "App installed successfully",
            "device_id": device_id,
            "app_path": app_path,
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceLaunchTool - Launch app on device
// ============================================================================

pub struct DeviceLaunchTool {
    schema: ToolSchema,
}

impl DeviceLaunchTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_launch".to_string(),
                aliases: Some(vec!["launch_app_device".to_string()]),
                description: "Launch an app on a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "App bundle identifier"
                        },
                        "arguments": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Launch arguments"
                        },
                        "environment": {
                            "type": "object",
                            "description": "Environment variables"
                        },
                        "wait_for_debugger": {
                            "type": "boolean",
                            "description": "Wait for debugger to attach (default: false)"
                        }
                    },
                    "required": ["device_id", "bundle_id"]
                }),
            },
        }
    }
}

impl Default for DeviceLaunchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceLaunchTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'device_id' is required".to_string()))?;

        let bundle_id = params
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'bundle_id' is required".to_string()))?;

        let wait_for_debugger = params
            .get("wait_for_debugger")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut args = vec![
            "device", "process", "launch", "--device", device_id, bundle_id,
        ];

        if wait_for_debugger {
            args.push("--start-stopped");
        }

        let launch_args: Vec<String>;
        if let Some(arguments) = params.get("arguments").and_then(|v| v.as_array()) {
            launch_args = arguments
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for arg in &launch_args {
                args.push(arg);
            }
        }

        let output = run_devicectl(&args).await?;

        Ok(json!({
            "success": true,
            "message": "App launched",
            "device_id": device_id,
            "bundle_id": bundle_id,
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceStopTool - Stop app on device
// ============================================================================

pub struct DeviceStopTool {
    schema: ToolSchema,
}

impl DeviceStopTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_stop".to_string(),
                aliases: Some(vec!["stop_app_device".to_string()]),
                description: "Stop a running app on a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "App bundle identifier"
                        },
                        "signal": {
                            "type": "string",
                            "enum": ["SIGTERM", "SIGKILL", "SIGINT"],
                            "description": "Signal to send (default: SIGTERM)"
                        }
                    },
                    "required": ["device_id", "bundle_id"]
                }),
            },
        }
    }
}

impl Default for DeviceStopTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceStopTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'device_id' is required".to_string()))?;

        let bundle_id = params
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'bundle_id' is required".to_string()))?;

        let signal = params
            .get("signal")
            .and_then(|v| v.as_str())
            .unwrap_or("SIGTERM");

        // First get the PID of the running app
        let list_output = run_devicectl(&[
            "device",
            "info",
            "processes",
            "--device",
            device_id,
            "--json-output",
            "/dev/stdout",
        ])
        .await?;

        let json_output: Value = serde_json::from_str(&list_output).unwrap_or(json!({}));

        if let Some(processes) = json_output
            .get("result")
            .and_then(|r| r.get("runningProcesses"))
            .and_then(|p| p.as_array())
        {
            for process in processes {
                if process.get("bundleID").and_then(|b| b.as_str()) == Some(bundle_id)
                    && let Some(pid) = process.get("processIdentifier").and_then(|p| p.as_i64())
                {
                    let pid_str = pid.to_string();
                    let _ = run_devicectl(&[
                        "device", "process", "signal", "--device", device_id, "--pid", &pid_str,
                        "--signal", signal,
                    ])
                    .await;

                    return Ok(json!({
                        "success": true,
                        "message": "App stopped",
                        "device_id": device_id,
                        "bundle_id": bundle_id,
                        "pid": pid,
                        "signal": signal
                    }));
                }
            }
        }

        Ok(json!({
            "success": false,
            "message": "App not found running on device",
            "device_id": device_id,
            "bundle_id": bundle_id
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceAppPathTool - Get installed app path on device
// ============================================================================

pub struct DeviceAppPathTool {
    schema: ToolSchema,
}

impl DeviceAppPathTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_app_path".to_string(),
                aliases: Some(vec!["get_device_app_path".to_string()]),
                description: "Get the filesystem path of an installed app on a physical device."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "App bundle identifier"
                        }
                    },
                    "required": ["device_id", "bundle_id"]
                }),
            },
        }
    }
}

impl Default for DeviceAppPathTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceAppPathTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'device_id' is required".to_string()))?;

        let bundle_id = params
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'bundle_id' is required".to_string()))?;

        // List installed apps to find the path
        let output = run_devicectl(&[
            "device",
            "info",
            "apps",
            "--device",
            device_id,
            "--json-output",
            "/dev/stdout",
        ])
        .await?;

        let json_output: Value = serde_json::from_str(&output).unwrap_or(json!({}));

        if let Some(apps) = json_output
            .get("result")
            .and_then(|r| r.get("apps"))
            .and_then(|a| a.as_array())
        {
            for app in apps {
                if app.get("bundleID").and_then(|b| b.as_str()) == Some(bundle_id) {
                    let path = app
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("unknown");
                    let version = app
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");

                    return Ok(json!({
                        "success": true,
                        "bundle_id": bundle_id,
                        "path": path,
                        "version": version,
                        "device_id": device_id
                    }));
                }
            }
        }

        Ok(json!({
            "success": false,
            "message": "App not found on device",
            "bundle_id": bundle_id,
            "device_id": device_id
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceLogStartTool - Start device log capture
// ============================================================================

pub struct DeviceLogStartTool {
    schema: ToolSchema,
}

impl DeviceLogStartTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_log_start".to_string(),
                aliases: Some(vec!["start_device_log_cap".to_string()]),
                description: "Start capturing logs from a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "process": {
                            "type": "string",
                            "description": "Filter by process name or bundle ID"
                        },
                        "level": {
                            "type": "string",
                            "enum": ["default", "info", "debug"],
                            "description": "Log level (default: default)"
                        }
                    },
                    "required": ["device_id"]
                }),
            },
        }
    }
}

impl Default for DeviceLogStartTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceLogStartTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'device_id' is required".to_string()))?;

        let process = params.get("process").and_then(|v| v.as_str());
        let level = params
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let mut args = vec![
            "devicectl".to_string(),
            "device".to_string(),
            "process".to_string(),
            "log".to_string(),
            "--device".to_string(),
            device_id.to_string(),
            "--level".to_string(),
            level.to_string(),
        ];

        if let Some(proc) = process {
            args.push("--process".to_string());
            args.push(proc.to_string());
        }

        let mut cmd = Command::new("xcrun");
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| TestError::Execution(format!("Failed to start log capture: {}", e)))?;

        let output_buffer = Arc::new(std::sync::Mutex::new(Vec::new()));
        let output_clone = Arc::clone(&output_buffer);

        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut buf) = output_clone.lock() {
                        buf.push(line);
                        if buf.len() > 5000 {
                            buf.drain(0..1000);
                        }
                    }
                }
            });
        }

        let session_id = Uuid::new_v4().to_string();
        let session = DeviceLogSession {
            child,
            output_buffer,
            device_id: device_id.to_string(),
            start_time: Instant::now(),
        };

        {
            let mut sessions = DEVICE_LOG_SESSIONS.lock().await;
            sessions.insert(session_id.clone(), session);
        }

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "device_id": device_id,
            "message": "Log capture started"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceLogStopTool - Stop device log capture
// ============================================================================

pub struct DeviceLogStopTool {
    schema: ToolSchema,
}

impl DeviceLogStopTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_log_stop".to_string(),
                aliases: Some(vec!["stop_device_log_cap".to_string()]),
                description: "Stop capturing logs from a physical device and return captured logs."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Log capture session ID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for DeviceLogStopTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceLogStopTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        let mut sessions = DEVICE_LOG_SESSIONS.lock().await;
        let session = sessions
            .remove(session_id)
            .ok_or_else(|| TestError::Execution(format!("Session '{}' not found", session_id)))?;

        let duration_secs = session.start_time.elapsed().as_secs();

        // Kill the log process
        let mut child = session.child;
        let _ = child.kill().await;

        // Get captured logs
        let logs = session
            .output_buffer
            .lock()
            .map(|buf| buf.clone())
            .unwrap_or_default();

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "device_id": session.device_id,
            "duration_secs": duration_secs,
            "log_count": logs.len(),
            "logs": logs
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceTestTool - Run tests on physical device
// ============================================================================

pub struct DeviceTestTool {
    schema: ToolSchema,
}

impl DeviceTestTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_test".to_string(),
                aliases: Some(vec!["test_device".to_string()]),
                description: "Run Xcode tests on a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace"
                        },
                        "scheme": {
                            "type": "string",
                            "description": "Test scheme name"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Target device UDID"
                        },
                        "test_plan": {
                            "type": "string",
                            "description": "Test plan name"
                        },
                        "only_testing": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Run only specific tests (e.g., 'MyTests/testExample')"
                        },
                        "skip_testing": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Skip specific tests"
                        },
                        "result_bundle_path": {
                            "type": "string",
                            "description": "Path to save test results (.xcresult)"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Test timeout in seconds (default: 600)"
                        }
                    },
                    "required": ["scheme", "device_id"]
                }),
            },
        }
    }
}

impl Default for DeviceTestTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceTestTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'device_id' is required".to_string()))?;

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(600);

        let destination = format!("platform=iOS,id={}", device_id);
        let mut args = vec![
            "test",
            "-scheme",
            scheme,
            "-destination",
            &destination,
            "-sdk",
            "iphoneos",
        ];

        let workspace;
        let project;
        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            workspace = ws.to_string();
            args.push("-workspace");
            args.push(&workspace);
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            project = proj.to_string();
            args.push("-project");
            args.push(&project);
        }

        let test_plan;
        if let Some(tp) = params.get("test_plan").and_then(|v| v.as_str()) {
            test_plan = tp.to_string();
            args.push("-testPlan");
            args.push(&test_plan);
        }

        let result_bundle;
        if let Some(rb) = params.get("result_bundle_path").and_then(|v| v.as_str()) {
            result_bundle = rb.to_string();
            args.push("-resultBundlePath");
            args.push(&result_bundle);
        }

        let only_testing: Vec<String>;
        if let Some(tests) = params.get("only_testing").and_then(|v| v.as_array()) {
            only_testing = tests
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for test in &only_testing {
                args.push("-only-testing");
                args.push(test);
            }
        }

        let skip_testing: Vec<String>;
        if let Some(tests) = params.get("skip_testing").and_then(|v| v.as_array()) {
            skip_testing = tests
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for test in &skip_testing {
                args.push("-skip-testing");
                args.push(test);
            }
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        // Parse test results from output
        let tests_passed = stdout.contains("Test Suite") && stdout.contains("passed");
        let test_count = stdout.lines().filter(|l| l.contains("Test Case")).count();

        Ok(json!({
            "success": success && tests_passed,
            "device_id": device_id,
            "scheme": scheme,
            "tests_run": test_count,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceCleanTool - Clean build artifacts
// ============================================================================

pub struct DeviceCleanTool {
    schema: ToolSchema,
}

impl DeviceCleanTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_clean".to_string(),
                aliases: Some(vec!["clean_device_build".to_string()]),
                description: "Clean Xcode build artifacts for device builds.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace"
                        },
                        "scheme": {
                            "type": "string",
                            "description": "Build scheme name"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for DeviceCleanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceCleanTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let mut args = vec!["clean", "-scheme", scheme, "-sdk", "iphoneos"];

        let workspace;
        let project;
        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            workspace = ws.to_string();
            args.push("-workspace");
            args.push(&workspace);
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            project = proj.to_string();
            args.push("-project");
            args.push(&project);
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, 120).await?;

        Ok(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_list_tool_schema() {
        let tool = DeviceListTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_list");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"list_devices".to_string())
        );
    }

    #[test]
    fn test_device_build_tool_schema() {
        let tool = DeviceBuildTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_build");
    }

    #[test]
    fn test_device_install_tool_schema() {
        let tool = DeviceInstallTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_install");
    }

    #[test]
    fn test_device_launch_tool_schema() {
        let tool = DeviceLaunchTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_launch");
    }

    #[test]
    fn test_device_stop_tool_schema() {
        let tool = DeviceStopTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_stop");
    }

    #[test]
    fn test_device_app_path_tool_schema() {
        let tool = DeviceAppPathTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_app_path");
    }

    #[test]
    fn test_device_log_start_tool_schema() {
        let tool = DeviceLogStartTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_log_start");
    }

    #[test]
    fn test_device_log_stop_tool_schema() {
        let tool = DeviceLogStopTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_log_stop");
    }

    #[test]
    fn test_device_test_tool_schema() {
        let tool = DeviceTestTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_test");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"test_device".to_string())
        );
    }

    #[test]
    fn test_device_clean_tool_schema() {
        let tool = DeviceCleanTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_clean");
    }
}
