use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub struct ScreenshotAnalyzer {
    schema: ToolSchema,
}

impl ScreenshotAnalyzer {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "analyze_screenshot".to_string(),
                description: "Analyze a screenshot and describe what you see. Returns the image path for vision model processing.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the screenshot file"
                        },
                        "prompt": {
                            "type": "string",
                            "description": "Optional prompt for what to focus on in the analysis",
                            "default": "Describe what you see in this screenshot, focusing on UI elements, their states, and any notable features."
                        }
                    },
                    "required": ["path"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ScreenshotAnalyzer {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing path parameter".to_string()))?;

        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe what you see in this screenshot, focusing on UI elements, their states, and any notable features.");

        // Verify the file exists
        if !Path::new(path).exists() {
            return Ok(json!({
                "error": {
                    "code": "FILE_NOT_FOUND",
                    "message": format!("Screenshot file not found: {}", path)
                }
            }));
        }

        // Verify it's an image file
        if !path.ends_with(".png") && !path.ends_with(".jpg") && !path.ends_with(".jpeg") {
            return Ok(json!({
                "error": {
                    "code": "INVALID_FILE_TYPE",
                    "message": "File must be a PNG or JPEG image"
                }
            }));
        }

        // Get file size for context
        let metadata = fs::metadata(path).map_err(|e| TestError::Mcp(format!("Failed to read file metadata: {}", e)))?;
        let file_size = metadata.len();

        Ok(json!({
            "success": true,
            "screenshot_path": path,
            "file_size_bytes": file_size,
            "analysis_prompt": prompt,
            "instructions": "The screenshot is ready for analysis. Use your vision capabilities to analyze the image at the provided path.",
            "note": "This tool prepares the screenshot for analysis. The actual visual analysis should be performed by the vision model using @screenshot command."
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}