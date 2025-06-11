use super::server::{Tool, ToolSchema};
use super::xcode_version::XcodeVersion;
use crate::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct XcodeInfoTool {
    schema: ToolSchema,
}

impl XcodeInfoTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_info".to_string(),
                description: "Get information about the installed Xcode version and available simulator features".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "check_features": {
                            "type": "boolean",
                            "default": true,
                            "description": "Include feature availability information"
                        }
                    }
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for XcodeInfoTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let check_features = params
            .get("check_features")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        match XcodeVersion::detect() {
            Ok(version) => {
                let mut result = json!({
                    "xcode_version": {
                        "major": version.major,
                        "minor": version.minor,
                        "patch": version.patch,
                        "display": format!("{}.{}.{}", version.major, version.minor, version.patch)
                    }
                });

                if check_features {
                    result["features"] = json!({
                        "bootstatus": {
                            "available": version.supports_bootstatus(),
                            "description": "Check simulator boot status",
                            "min_version": "11.0.0"
                        },
                        "privacy": {
                            "available": version.supports_privacy(),
                            "description": "Manage privacy permissions",
                            "min_version": "11.4.0"
                        },
                        "ui_commands": {
                            "available": version.supports_ui_commands(),
                            "description": "Basic UI automation commands",
                            "min_version": "15.0.0"
                        },
                        "device_appearance": {
                            "available": version.supports_device_appearance(),
                            "description": "Set device appearance (light/dark mode)",
                            "min_version": "13.0.0"
                        },
                        "push_notification": {
                            "available": version.supports_push_notification(),
                            "description": "Send push notifications to simulator",
                            "min_version": "11.4.0"
                        },
                        "clone": {
                            "available": version.supports_clone(),
                            "description": "Clone simulator devices",
                            "min_version": "12.0.0"
                        },
                        "device_pair": {
                            "available": version.supports_device_pair(),
                            "description": "Pair simulators for multi-device testing",
                            "min_version": "14.0.0"
                        },
                        "device_focus": {
                            "available": version.supports_device_focus(),
                            "description": "Focus mode for reducing distractions",
                            "min_version": "16.0.0"
                        },
                        "device_streaming": {
                            "available": version.supports_device_streaming(),
                            "description": "Stream device screen",
                            "min_version": "25.0.0"
                        },
                        "enhanced_ui_interaction": {
                            "available": version.supports_enhanced_ui_interaction(),
                            "description": "Enhanced UI interaction capabilities",
                            "min_version": "26.0.0"
                        }
                    });

                    // Add compatibility warnings
                    let mut warnings = Vec::new();
                    
                    if version.major < 15 {
                        warnings.push("UI automation features are limited. Consider upgrading to Xcode 15 or later.");
                    }
                    
                    if version.major >= 26 {
                        warnings.push("Running Xcode 26 beta. New features are being integrated.");
                    }
                    
                    if !warnings.is_empty() {
                        result["warnings"] = json!(warnings);
                    }
                }

                Ok(result)
            }
            Err(e) => Ok(json!({
                "error": {
                    "code": "XCODE_DETECTION_FAILED",
                    "message": e.to_string(),
                    "suggestion": "Ensure Xcode is installed and xcode-select is configured correctly"
                }
            }))
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

impl Default for XcodeInfoTool {
    fn default() -> Self {
        Self::new()
    }
}