use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
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
        .map_err(|e| TestError::Execution(format!("Failed to run simctl: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TestError::Execution(format!("simctl failed: {}", stderr)));
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
                aliases: Some(vec![
                    "list_simulators".to_string(),
                    "simulators".to_string(),
                ]),
                description: "List available iOS simulators with status, runtime, and device type."
                    .to_string(),
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
                TestError::Mcp("Either 'simulator_id' or 'simulator_name' is required".to_string())
            })?;

        let output = run_simctl(&["list", "-j", "devices"]).await?;
        let simulators = parse_simctl_list(&output);

        let sim = find_simulator(&simulators, id_or_name)
            .ok_or_else(|| TestError::Execution(format!("Simulator '{}' not found", id_or_name)))?;

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
        let shutdown_all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

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
                TestError::Mcp(
                    "Either 'simulator_id', 'simulator_name', or 'all: true' is required"
                        .to_string(),
                )
            })?;

        let output = run_simctl(&["list", "-j", "devices"]).await?;
        let simulators = parse_simctl_list(&output);

        let sim = find_simulator(&simulators, id_or_name)
            .ok_or_else(|| TestError::Execution(format!("Simulator '{}' not found", id_or_name)))?;

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

        let image_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("png");

        let args = vec![
            "io",
            simulator_id,
            "screenshot",
            "--type",
            image_type,
            output_path,
        ];

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

// ============================================================================
// SimSetLocationTool - Set simulator GPS location
// ============================================================================

pub struct SimSetLocationTool {
    schema: ToolSchema,
}

impl SimSetLocationTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_set_location".to_string(),
                aliases: Some(vec!["set_location".to_string(), "gps".to_string()]),
                description: "Set GPS location on an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "latitude": {
                            "type": "number",
                            "description": "Latitude coordinate"
                        },
                        "longitude": {
                            "type": "number",
                            "description": "Longitude coordinate"
                        }
                    },
                    "required": ["latitude", "longitude"]
                }),
            },
        }
    }
}

impl Default for SimSetLocationTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimSetLocationTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let latitude = params
            .get("latitude")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| TestError::Mcp("'latitude' is required".to_string()))?;

        let longitude = params
            .get("longitude")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| TestError::Mcp("'longitude' is required".to_string()))?;

        let lat_str = latitude.to_string();
        let lon_str = longitude.to_string();

        run_simctl(&["location", simulator_id, "set", &lat_str, &lon_str]).await?;

        Ok(json!({
            "success": true,
            "message": "Location set",
            "location": {
                "latitude": latitude,
                "longitude": longitude
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SimResetLocationTool - Reset simulator location
// ============================================================================

pub struct SimResetLocationTool {
    schema: ToolSchema,
}

impl SimResetLocationTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_reset_location".to_string(),
                aliases: Some(vec!["reset_location".to_string()]),
                description: "Reset GPS location on an iOS simulator to default.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SimResetLocationTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimResetLocationTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        run_simctl(&["location", simulator_id, "clear"]).await?;

        Ok(json!({
            "success": true,
            "message": "Location reset to default"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SimSetAppearanceTool - Set dark/light mode
// ============================================================================

pub struct SimSetAppearanceTool {
    schema: ToolSchema,
}

impl SimSetAppearanceTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_set_appearance".to_string(),
                aliases: Some(vec!["appearance".to_string(), "dark_mode".to_string()]),
                description: "Set appearance mode (dark/light) on an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["dark", "light"],
                            "description": "Appearance mode"
                        }
                    },
                    "required": ["mode"]
                }),
            },
        }
    }
}

impl Default for SimSetAppearanceTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimSetAppearanceTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let mode = params
            .get("mode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'mode' is required (dark or light)".to_string()))?;

        run_simctl(&["ui", simulator_id, "appearance", mode]).await?;

        Ok(json!({
            "success": true,
            "message": format!("Appearance set to {}", mode),
            "mode": mode
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SimEraseTool - Erase (factory reset) simulator
// ============================================================================

pub struct SimEraseTool {
    schema: ToolSchema,
}

impl SimEraseTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_erase".to_string(),
                aliases: Some(vec![
                    "erase_simulator".to_string(),
                    "reset_simulator".to_string(),
                ]),
                description: "Erase (factory reset) an iOS simulator, removing all data."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID"
                        },
                        "all": {
                            "type": "boolean",
                            "description": "Erase all simulators (default: false)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SimEraseTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimEraseTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let erase_all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        if erase_all {
            run_simctl(&["erase", "all"]).await?;
            return Ok(json!({
                "success": true,
                "message": "All simulators erased"
            }));
        }

        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TestError::Mcp("Either 'simulator_id' or 'all: true' is required".to_string())
            })?;

        run_simctl(&["erase", simulator_id]).await?;

        Ok(json!({
            "success": true,
            "message": format!("Simulator {} erased", simulator_id)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SimOpenTool - Open Simulator.app GUI
// ============================================================================

pub struct SimOpenTool {
    schema: ToolSchema,
}

impl SimOpenTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sim_open".to_string(),
                aliases: Some(vec!["open_simulator".to_string()]),
                description: "Open the Simulator.app GUI, optionally for a specific simulator."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID to open (optional)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SimOpenTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SimOpenTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        Command::new("open")
            .arg("-a")
            .arg("Simulator")
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to open Simulator: {}", e)))?;

        if let Some(simulator_id) = params.get("simulator_id").and_then(|v| v.as_str()) {
            run_simctl(&["boot", simulator_id]).await.ok();
        }

        Ok(json!({
            "success": true,
            "message": "Simulator.app opened"
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
    fn test_sim_list_tool_schema() {
        let tool = SimListTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_list");
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
    }
}
