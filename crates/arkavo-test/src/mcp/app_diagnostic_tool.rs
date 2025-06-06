use crate::mcp::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::process::Command;

pub struct AppDiagnosticTool {
    schema: ToolSchema,
}

impl AppDiagnosticTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "app_diagnostic".to_string(),
                description: "Check if apps are installed and get their state on the simulator"
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Simulator device ID"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Optional app bundle ID to check specifically"
                        }
                    },
                    "required": ["device_id"]
                }),
            },
        }
    }
}

impl Default for AppDiagnosticTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for AppDiagnosticTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("device_id is required".to_string()))?;

        let bundle_id = params.get("bundle_id").and_then(|v| v.as_str());

        // List all installed apps
        let list_output = Command::new("xcrun")
            .args(["simctl", "listapps", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list apps: {}", e)))?;

        if !list_output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to list apps: {}",
                String::from_utf8_lossy(&list_output.stderr)
            )));
        }

        let apps_json = String::from_utf8_lossy(&list_output.stdout);

        // Parse the JSON to extract app info
        let apps: Value = serde_json::from_str(&apps_json)
            .map_err(|e| TestError::Mcp(format!("Failed to parse apps JSON: {}", e)))?;

        // If specific bundle ID requested, find it
        if let Some(target_bundle_id) = bundle_id {
            if let Some(apps_array) = apps.as_object() {
                for (_, app) in apps_array {
                    if let Some(app_bundle_id) =
                        app.get("CFBundleIdentifier").and_then(|v| v.as_str())
                    {
                        if app_bundle_id == target_bundle_id {
                            return Ok(serde_json::json!({
                                "found": true,
                                "bundle_id": target_bundle_id,
                                "app_info": app,
                                "status": "installed"
                            }));
                        }
                    }
                }
            }

            return Ok(serde_json::json!({
                "found": false,
                "bundle_id": target_bundle_id,
                "status": "not_installed",
                "message": format!("App '{}' is not installed on the simulator. You need to install it first.", target_bundle_id),
                "available_apps": apps.as_object().map(|a| {
                    a.values()
                        .filter_map(|app| app.get("CFBundleIdentifier").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                }).unwrap_or_default()
            }));
        }

        // Return all apps
        Ok(serde_json::json!({
            "device_id": device_id,
            "installed_apps": apps,
            "app_count": apps.as_object().map(|a| a.len()).unwrap_or(0)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
