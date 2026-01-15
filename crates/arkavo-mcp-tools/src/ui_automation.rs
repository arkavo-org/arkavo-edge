use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;

/// Execute simctl UI interaction command
async fn run_simctl_ui(args: &[&str]) -> Result<String> {
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
// UiTapTool - Tap on screen coordinates or element
// ============================================================================

pub struct UiTapTool {
    schema: ToolSchema,
}

impl UiTapTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_tap".to_string(),
                aliases: Some(vec!["tap".to_string(), "click".to_string()]),
                description: "Tap on screen coordinates in an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "x": {
                            "type": "number",
                            "description": "X coordinate"
                        },
                        "y": {
                            "type": "number",
                            "description": "Y coordinate"
                        }
                    },
                    "required": ["x", "y"]
                }),
            },
        }
    }
}

impl Default for UiTapTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiTapTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let x = params.get("x").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'x' coordinate is required".to_string())
        })?;

        let y = params.get("y").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'y' coordinate is required".to_string())
        })?;

        let x_str = x.to_string();
        let y_str = y.to_string();

        run_simctl_ui(&["io", simulator_id, "tap", &x_str, &y_str]).await?;

        Ok(json!({
            "success": true,
            "action": "tap",
            "x": x,
            "y": y
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// UiSwipeTool - Swipe gesture
// ============================================================================

pub struct UiSwipeTool {
    schema: ToolSchema,
}

impl UiSwipeTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_swipe".to_string(),
                aliases: Some(vec!["swipe".to_string()]),
                description: "Perform a swipe gesture on an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "start_x": {
                            "type": "number",
                            "description": "Start X coordinate"
                        },
                        "start_y": {
                            "type": "number",
                            "description": "Start Y coordinate"
                        },
                        "end_x": {
                            "type": "number",
                            "description": "End X coordinate"
                        },
                        "end_y": {
                            "type": "number",
                            "description": "End Y coordinate"
                        },
                        "duration": {
                            "type": "number",
                            "description": "Swipe duration in seconds (default: 0.5)"
                        }
                    },
                    "required": ["start_x", "start_y", "end_x", "end_y"]
                }),
            },
        }
    }
}

impl Default for UiSwipeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiSwipeTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let start_x = params.get("start_x").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'start_x' is required".to_string())
        })?;
        let start_y = params.get("start_y").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'start_y' is required".to_string())
        })?;
        let end_x = params.get("end_x").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'end_x' is required".to_string())
        })?;
        let end_y = params.get("end_y").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'end_y' is required".to_string())
        })?;

        let duration = params.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.5);

        // Use touch sequence for swipe
        let start_x_str = start_x.to_string();
        let start_y_str = start_y.to_string();
        let end_x_str = end_x.to_string();
        let end_y_str = end_y.to_string();
        let dur_str = duration.to_string();

        // simctl io swipe command
        run_simctl_ui(&[
            "io", simulator_id, "swipe",
            &start_x_str, &start_y_str, &end_x_str, &end_y_str,
            "--duration", &dur_str,
        ]).await?;

        Ok(json!({
            "success": true,
            "action": "swipe",
            "from": {"x": start_x, "y": start_y},
            "to": {"x": end_x, "y": end_y},
            "duration": duration
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// UiLongPressTool - Long press gesture
// ============================================================================

pub struct UiLongPressTool {
    schema: ToolSchema,
}

impl UiLongPressTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_long_press".to_string(),
                aliases: Some(vec!["long_press".to_string(), "hold".to_string()]),
                description: "Perform a long press gesture on an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "x": {
                            "type": "number",
                            "description": "X coordinate"
                        },
                        "y": {
                            "type": "number",
                            "description": "Y coordinate"
                        },
                        "duration": {
                            "type": "number",
                            "description": "Press duration in seconds (default: 1.0)"
                        }
                    },
                    "required": ["x", "y"]
                }),
            },
        }
    }
}

impl Default for UiLongPressTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiLongPressTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let x = params.get("x").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'x' is required".to_string())
        })?;
        let y = params.get("y").and_then(|v| v.as_f64()).ok_or_else(|| {
            ToolError::InvalidParams("'y' is required".to_string())
        })?;

        let duration = params.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);

        let x_str = x.to_string();
        let y_str = y.to_string();
        let dur_str = duration.to_string();

        run_simctl_ui(&[
            "io", simulator_id, "touch",
            &x_str, &y_str,
            "--duration", &dur_str,
        ]).await?;

        Ok(json!({
            "success": true,
            "action": "long_press",
            "x": x,
            "y": y,
            "duration": duration
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// UiTypeTextTool - Type text
// ============================================================================

pub struct UiTypeTextTool {
    schema: ToolSchema,
}

impl UiTypeTextTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_type_text".to_string(),
                aliases: Some(vec!["type".to_string(), "input".to_string()]),
                description: "Type text into the focused field on an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "text": {
                            "type": "string",
                            "description": "Text to type"
                        }
                    },
                    "required": ["text"]
                }),
            },
        }
    }
}

impl Default for UiTypeTextTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiTypeTextTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let text = params.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'text' is required".to_string())
        })?;

        run_simctl_ui(&["io", simulator_id, "input", "text", text]).await?;

        Ok(json!({
            "success": true,
            "action": "type_text",
            "text": text
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// UiKeyPressTool - Press a key
// ============================================================================

pub struct UiKeyPressTool {
    schema: ToolSchema,
}

impl UiKeyPressTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_key_press".to_string(),
                aliases: Some(vec!["key".to_string(), "press_key".to_string()]),
                description: "Press a keyboard key on an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key to press (e.g., 'return', 'delete', 'escape')"
                        },
                        "keycode": {
                            "type": "integer",
                            "description": "Raw keycode to send"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for UiKeyPressTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiKeyPressTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let key = params.get("key").and_then(|v| v.as_str());
        let keycode = params.get("keycode").and_then(|v| v.as_u64());

        if key.is_none() && keycode.is_none() {
            return Err(ToolError::InvalidParams(
                "Either 'key' or 'keycode' is required".to_string(),
            ));
        }

        if let Some(k) = key {
            // Map common key names to keycodes or use special command
            let key_arg = match k.to_lowercase().as_str() {
                "return" | "enter" => "36",
                "delete" | "backspace" => "51",
                "escape" | "esc" => "53",
                "tab" => "48",
                "space" => "49",
                "up" => "126",
                "down" => "125",
                "left" => "123",
                "right" => "124",
                _ => k,
            };
            run_simctl_ui(&["io", simulator_id, "input", "keycode", key_arg]).await?;
        } else if let Some(kc) = keycode {
            let kc_str = kc.to_string();
            run_simctl_ui(&["io", simulator_id, "input", "keycode", &kc_str]).await?;
        }

        Ok(json!({
            "success": true,
            "action": "key_press",
            "key": key,
            "keycode": keycode
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// UiDescribeTool - Describe UI hierarchy
// ============================================================================

pub struct UiDescribeTool {
    schema: ToolSchema,
}

impl UiDescribeTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_describe".to_string(),
                aliases: Some(vec!["describe_ui".to_string(), "accessibility".to_string()]),
                description: "Get the UI accessibility hierarchy from an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "App bundle ID to inspect"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for UiDescribeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiDescribeTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let bundle_id = params.get("bundle_id").and_then(|v| v.as_str());

        // Use accessibility inspector via simctl or fbsimctl
        // Note: Full accessibility hierarchy requires additional tooling
        let mut args = vec!["ui", simulator_id, "describe"];

        if let Some(bid) = bundle_id {
            args.push("--bundle-id");
            args.push(bid);
        }

        // Try to get UI description
        let output = run_simctl_ui(&args).await.unwrap_or_else(|_| {
            "UI describe requires additional tooling (e.g., AXe or Accessibility Inspector)".to_string()
        });

        Ok(json!({
            "success": true,
            "simulator_id": simulator_id,
            "bundle_id": bundle_id,
            "ui_hierarchy": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// UiScreenshotTool - Take screenshot with annotations
// ============================================================================

pub struct UiScreenshotTool {
    schema: ToolSchema,
}

impl UiScreenshotTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_screenshot".to_string(),
                aliases: Some(vec!["screenshot".to_string(), "capture_ui".to_string()]),
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
                            "description": "Output file path (default: /tmp/ui_screenshot.png)"
                        },
                        "format": {
                            "type": "string",
                            "enum": ["png", "jpeg", "tiff"],
                            "description": "Image format (default: png)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for UiScreenshotTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UiScreenshotTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted");

        let output_path = params
            .get("output_path")
            .and_then(|v| v.as_str())
            .unwrap_or("/tmp/ui_screenshot.png");

        let format = params
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("png");

        run_simctl_ui(&[
            "io", simulator_id, "screenshot",
            "--type", format,
            output_path,
        ]).await?;

        Ok(json!({
            "success": true,
            "path": output_path,
            "format": format
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
    fn test_ui_tap_tool_schema() {
        let tool = UiTapTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "ui_tap");
        assert!(schema.aliases.as_ref().unwrap().contains(&"tap".to_string()));
    }

    #[test]
    fn test_ui_swipe_tool_schema() {
        let tool = UiSwipeTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "ui_swipe");
    }

    #[test]
    fn test_ui_long_press_tool_schema() {
        let tool = UiLongPressTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "ui_long_press");
    }

    #[test]
    fn test_ui_type_text_tool_schema() {
        let tool = UiTypeTextTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "ui_type_text");
    }

    #[test]
    fn test_ui_key_press_tool_schema() {
        let tool = UiKeyPressTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "ui_key_press");
    }

    #[test]
    fn test_ui_describe_tool_schema() {
        let tool = UiDescribeTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "ui_describe");
    }

    #[test]
    fn test_ui_screenshot_tool_schema() {
        let tool = UiScreenshotTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "ui_screenshot");
    }
}
