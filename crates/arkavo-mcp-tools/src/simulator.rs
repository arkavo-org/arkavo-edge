use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;

/// Simulator information returned by simctl
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatorInfo {
    pub udid: String,
    pub name: String,
    pub state: String,
    pub runtime: String,
    pub device_type: String,
    pub is_available: bool,
}

/// Parse simctl list output into structured data
fn parse_simctl_list(output: &str) -> Vec<SimulatorInfo> {
    let mut simulators = Vec::new();

    // Parse JSON output from simctl list -j
    if let Ok(json_value) = serde_json::from_str::<Value>(output) {
        if let Some(devices) = json_value.get("devices").and_then(|d| d.as_object()) {
            for (runtime, device_list) in devices {
                if let Some(devices_array) = device_list.as_array() {
                    for device in devices_array {
                        if let (Some(udid), Some(name), Some(state)) = (
                            device.get("udid").and_then(|v| v.as_str()),
                            device.get("name").and_then(|v| v.as_str()),
                            device.get("state").and_then(|v| v.as_str()),
                        ) {
                            let is_available = device
                                .get("isAvailable")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let device_type = device
                                .get("deviceTypeIdentifier")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown")
                                .to_string();

                            simulators.push(SimulatorInfo {
                                udid: udid.to_string(),
                                name: name.to_string(),
                                state: state.to_string(),
                                runtime: runtime.clone(),
                                device_type,
                                is_available,
                            });
                        }
                    }
                }
            }
        }
    }
    simulators
}

/// Execute simctl command and return output
async fn run_simctl(args: &[&str]) -> Result<String> {
    let output = Command::new("xcrun")
        .arg("simctl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ToolError::Execution(format!("Failed to run simctl: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::Execution(format!("simctl failed: {}", stderr)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Find simulator by ID or name
fn find_simulator(simulators: &[SimulatorInfo], id_or_name: &str) -> Option<SimulatorInfo> {
    simulators
        .iter()
        .find(|s| s.udid == id_or_name || s.name.to_lowercase() == id_or_name.to_lowercase())
        .cloned()
}

// ============================================================================
// SimListTool - List available iOS simulators
// ============================================================================

pub struct SimListTool {
    schema: ToolSchema,
}

impl SimListTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_list".to_string(),
                aliases: Some(vec!["list_simulators".to_string(), "simulators".to_string()]),
                description: "List available iOS simulators with their status, runtime, and device type.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "available_only": {
                            "type": "boolean",
                            "description": "Only show available simulators (default: false)"
                        },
                        "state": {
                            "type": "string",
                            "enum": ["Booted", "Shutdown", "Creating"],
                            "description": "Filter by simulator state"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SimListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimListTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let output = run_simctl(&["list", "-j", "devices"]).await?;
        let mut simulators = parse_simctl_list(&output);

        // Apply filters
        let available_only = params
            .get("available_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if available_only {
            simulators.retain(|s| s.is_available);
        }

        if let Some(state) = params.get("state").and_then(|v| v.as_str()) {
            simulators.retain(|s| s.state.to_lowercase() == state.to_lowercase());
        }

        Ok(json!({
            "success": true,
            "count": simulators.len(),
            "simulators": simulators
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SimBootTool - Boot a simulator
// ============================================================================

pub struct SimBootTool {
    schema: ToolSchema,
}

impl SimBootTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_boot".to_string(),
                aliases: Some(vec!["boot_simulator".to_string()]),
                description: "Boot an iOS simulator by UUID or name.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID"
                        },
                        "simulator_name": {
                            "type": "string",
                            "description": "Simulator name (e.g., 'iPhone 15 Pro')"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SimBootTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimBootTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let id_or_name = params
            .get("simulator_id")
            .or_else(|| params.get("simulator_name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParams(
                    "Either 'simulator_id' or 'simulator_name' is required".to_string(),
                )
            })?;

        // Get list to find the simulator
        let output = run_simctl(&["list", "-j", "devices"]).await?;
        let simulators = parse_simctl_list(&output);

        let sim = find_simulator(&simulators, id_or_name).ok_or_else(|| {
            ToolError::Execution(format!("Simulator '{}' not found", id_or_name))
        })?;

        if sim.state == "Booted" {
            return Ok(json!({
                "success": true,
                "message": "Simulator is already booted",
                "simulator": sim
            }));
        }

        run_simctl(&["boot", &sim.udid]).await?;

        Ok(json!({
            "success": true,
            "message": format!("Simulator '{}' booted successfully", sim.name),
            "simulator": {
                "udid": sim.udid,
                "name": sim.name,
                "state": "Booted"
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SimShutdownTool - Shutdown a simulator
// ============================================================================

pub struct SimShutdownTool {
    schema: ToolSchema,
}

impl SimShutdownTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_shutdown".to_string(),
                aliases: Some(vec!["shutdown_simulator".to_string()]),
                description: "Shutdown a running iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID"
                        },
                        "simulator_name": {
                            "type": "string",
                            "description": "Simulator name"
                        },
                        "all": {
                            "type": "boolean",
                            "description": "Shutdown all simulators (default: false)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SimShutdownTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimShutdownTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let shutdown_all = params
            .get("all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if shutdown_all {
            run_simctl(&["shutdown", "all"]).await?;
            return Ok(json!({
                "success": true,
                "message": "All simulators shut down"
            }));
        }

        let id_or_name = params
            .get("simulator_id")
            .or_else(|| params.get("simulator_name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParams(
                    "Either 'simulator_id', 'simulator_name', or 'all: true' is required".to_string(),
                )
            })?;

        let output = run_simctl(&["list", "-j", "devices"]).await?;
        let simulators = parse_simctl_list(&output);

        let sim = find_simulator(&simulators, id_or_name).ok_or_else(|| {
            ToolError::Execution(format!("Simulator '{}' not found", id_or_name))
        })?;

        run_simctl(&["shutdown", &sim.udid]).await?;

        Ok(json!({
            "success": true,
            "message": format!("Simulator '{}' shut down", sim.name),
            "simulator": {
                "udid": sim.udid,
                "name": sim.name,
                "state": "Shutdown"
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SimScreenshotTool - Capture simulator screenshot
// ============================================================================

pub struct SimScreenshotTool {
    schema: ToolSchema,
}

impl SimScreenshotTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_screenshot".to_string(),
                aliases: Some(vec!["screenshot".to_string(), "capture".to_string()]),
                description: "Capture a screenshot from an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "output_path": {
                            "type": "string",
                            "description": "Output file path (default: /tmp/simulator_screenshot.png)"
                        },
                        "type": {
                            "type": "string",
                            "enum": ["png", "tiff", "bmp", "gif", "jpeg"],
                            "description": "Image format (default: png)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SimScreenshotTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimScreenshotTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let output_path = params
            .get("output_path")
            .and_then(|v| v.as_str())
            .unwrap_or("/tmp/simulator_screenshot.png");

        let image_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("png");

        let mut args = vec!["io", simulator_id, "screenshot"];
        args.push("--type");
        args.push(image_type);
        args.push(output_path);

        run_simctl(&args).await?;

        Ok(json!({
            "success": true,
            "message": "Screenshot captured",
            "path": output_path,
            "format": image_type
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
    fn test_parse_simctl_list() {
        let json_output = r#"{
            "devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
                    {
                        "udid": "ABC-123",
                        "name": "iPhone 15 Pro",
                        "state": "Shutdown",
                        "isAvailable": true,
                        "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-15-Pro"
                    }
                ]
            }
        }"#;

        let simulators = parse_simctl_list(json_output);
        assert_eq!(simulators.len(), 1);
        assert_eq!(simulators[0].name, "iPhone 15 Pro");
        assert_eq!(simulators[0].udid, "ABC-123");
        assert_eq!(simulators[0].state, "Shutdown");
        assert!(simulators[0].is_available);
    }

    #[test]
    fn test_find_simulator_by_udid() {
        let simulators = vec![SimulatorInfo {
            udid: "ABC-123".to_string(),
            name: "iPhone 15 Pro".to_string(),
            state: "Shutdown".to_string(),
            runtime: "iOS-17-0".to_string(),
            device_type: "iPhone".to_string(),
            is_available: true,
        }];

        let found = find_simulator(&simulators, "ABC-123");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "iPhone 15 Pro");
    }

    #[test]
    fn test_find_simulator_by_name() {
        let simulators = vec![SimulatorInfo {
            udid: "ABC-123".to_string(),
            name: "iPhone 15 Pro".to_string(),
            state: "Shutdown".to_string(),
            runtime: "iOS-17-0".to_string(),
            device_type: "iPhone".to_string(),
            is_available: true,
        }];

        let found = find_simulator(&simulators, "iphone 15 pro");
        assert!(found.is_some());
        assert_eq!(found.unwrap().udid, "ABC-123");
    }

    #[test]
    fn test_sim_list_tool_schema() {
        let tool = SimListTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_list");
        assert!(schema.aliases.as_ref().unwrap().contains(&"simulators".to_string()));
    }

    #[test]
    fn test_sim_boot_tool_schema() {
        let tool = SimBootTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_boot");
    }

    #[test]
    fn test_sim_shutdown_tool_schema() {
        let tool = SimShutdownTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_shutdown");
    }

    #[test]
    fn test_sim_screenshot_tool_schema() {
        let tool = SimScreenshotTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_screenshot");
        assert!(schema.aliases.as_ref().unwrap().contains(&"screenshot".to_string()));
    }
}
