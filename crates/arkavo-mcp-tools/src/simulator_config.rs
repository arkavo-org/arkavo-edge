use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;

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
            .ok_or_else(|| ToolError::InvalidParams("'latitude' is required".to_string()))?;

        let longitude = params
            .get("longitude")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::InvalidParams("'longitude' is required".to_string()))?;

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

        let mode = params.get("mode").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'mode' is required (dark or light)".to_string())
        })?;

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
                aliases: Some(vec!["erase_simulator".to_string(), "reset_simulator".to_string()]),
                description: "Erase (factory reset) an iOS simulator, removing all data.".to_string(),
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
                ToolError::InvalidParams(
                    "Either 'simulator_id' or 'all: true' is required".to_string(),
                )
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
                description: "Open the Simulator.app GUI, optionally for a specific simulator.".to_string(),
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
        // First open Simulator.app
        Command::new("open")
            .arg("-a")
            .arg("Simulator")
            .output()
            .await
            .map_err(|e| ToolError::Execution(format!("Failed to open Simulator: {}", e)))?;

        // If a specific simulator is requested, boot it
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
    fn test_sim_set_location_schema() {
        let tool = SimSetLocationTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_set_location");
        assert!(schema.aliases.as_ref().unwrap().contains(&"gps".to_string()));
    }

    #[test]
    fn test_sim_set_appearance_schema() {
        let tool = SimSetAppearanceTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_set_appearance");
        assert!(schema.aliases.as_ref().unwrap().contains(&"dark_mode".to_string()));
    }

    #[test]
    fn test_sim_erase_schema() {
        let tool = SimEraseTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_erase");
    }

    #[test]
    fn test_sim_open_schema() {
        let tool = SimOpenTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_open");
    }

    #[test]
    fn test_sim_reset_location_schema() {
        let tool = SimResetLocationTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "sim_reset_location");
    }
}
