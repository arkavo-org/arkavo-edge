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

        // simctl listapps outputs in plist format, not JSON
        // We'll use text parsing instead
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

        let output_text = String::from_utf8_lossy(&list_output.stdout);

        // Parse the plist-style output to find apps
        if let Some(target_bundle_id) = bundle_id {
            // Look for the specific bundle ID in the output
            let found = output_text.contains(&format!("\"{}\"", target_bundle_id)) ||
                       output_text.contains(&format!("{} =", target_bundle_id));
            
            // Try to extract some info about the app
            let mut app_info = serde_json::json!({});
            
            if found {
                // Look for the app's display name
                if let Some(start) = output_text.find(&format!("\"{}\"", target_bundle_id)) {
                    let app_section = &output_text[start..];
                    if let Some(name_start) = app_section.find("CFBundleDisplayName = ") {
                        let name_section = &app_section[name_start + 22..];
                        if let Some(name_end) = name_section.find(';') {
                            let display_name = name_section[..name_end].trim();
                            app_info["display_name"] = serde_json::json!(display_name);
                        }
                    }
                }
            }
            
            return Ok(serde_json::json!({
                "found": found,
                "bundle_id": target_bundle_id,
                "status": if found { "installed" } else { "not_installed" },
                "message": if found {
                    format!("App '{}' is installed on the simulator", target_bundle_id)
                } else {
                    format!("App '{}' is not installed on the simulator. You need to install it first.", target_bundle_id)
                },
                "app_info": app_info
            }));
        }

        // Count total apps if no specific bundle ID requested
        let app_count = output_text.matches(" = ").count() / 10; // Rough estimate
        
        Ok(serde_json::json!({
            "device_id": device_id,
            "status": "success",
            "message": "Use bundle_id parameter to check a specific app",
            "estimated_app_count": app_count,
            "note": "Full app listing not available in text format. Specify a bundle_id to check if a specific app is installed."
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
