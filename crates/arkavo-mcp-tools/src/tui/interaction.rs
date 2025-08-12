use super::keyboard::TuiKeyboardKit;
use crate::server::Tool;
use crate::tui::TuiScreenshotKit;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::sleep;

pub struct TuiInteractionKit {
    keyboard: TuiKeyboardKit,
    screenshot: TuiScreenshotKit,
    schema: ToolSchema,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action")]
enum InteractionAction {
    #[serde(rename = "navigate")]
    Navigate {
        direction: String,
        #[serde(default = "default_count")]
        count: u32,
    },
    #[serde(rename = "type_and_verify")]
    TypeAndVerify {
        text: String,
        #[serde(default = "default_verify_delay")]
        verify_delay_ms: u64,
        expected_content: Option<String>,
    },
    #[serde(rename = "shortcut_and_capture")]
    ShortcutAndCapture {
        shortcut: String,
        #[serde(default)]
        modifiers: Vec<String>,
        #[serde(default = "default_capture_delay")]
        capture_delay_ms: u64,
    },
    #[serde(rename = "verify_state")]
    VerifyState {
        expected_content: Option<String>,
        contains_text: Option<Vec<String>>,
        #[serde(default = "default_retry_count")]
        retry_count: u32,
        #[serde(default = "default_retry_delay")]
        retry_delay_ms: u64,
    },
    #[serde(rename = "clear_input")]
    ClearInput {
        #[serde(default = "default_select_all")]
        select_all: bool,
    },
}

fn default_count() -> u32 {
    1
}

fn default_verify_delay() -> u64 {
    500
}

fn default_capture_delay() -> u64 {
    1000
}

fn default_retry_count() -> u32 {
    3
}

fn default_retry_delay() -> u64 {
    1000
}

fn default_select_all() -> bool {
    true
}

impl Default for TuiInteractionKit {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiInteractionKit {
    pub fn new() -> Self {
        Self {
            keyboard: TuiKeyboardKit::new(),
            screenshot: TuiScreenshotKit::new(),
            schema: ToolSchema {
                name: "tui_interaction".to_string(),
                description:
                    "Comprehensive TUI interaction combining keyboard input and verification"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "navigate"
                                },
                                "direction": {
                                    "type": "string",
                                    "enum": ["up", "down", "left", "right", "home", "end", "pageup", "pagedown"]
                                },
                                "count": {
                                    "type": "integer",
                                    "minimum": 1,
                                    "default": 1
                                }
                            },
                            "required": ["action", "direction"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "type_and_verify"
                                },
                                "text": {
                                    "type": "string"
                                },
                                "verify_delay_ms": {
                                    "type": "integer",
                                    "default": 500
                                },
                                "expected_content": {
                                    "type": "string",
                                    "description": "Optional text to verify appears after typing"
                                }
                            },
                            "required": ["action", "text"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "shortcut_and_capture"
                                },
                                "shortcut": {
                                    "type": "string"
                                },
                                "modifiers": {
                                    "type": "array",
                                    "items": {
                                        "type": "string",
                                        "enum": ["ctrl", "alt", "shift", "cmd"]
                                    },
                                    "default": ["ctrl"]
                                },
                                "capture_delay_ms": {
                                    "type": "integer",
                                    "default": 1000
                                }
                            },
                            "required": ["action", "shortcut"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "verify_state"
                                },
                                "expected_content": {
                                    "type": "string"
                                },
                                "contains_text": {
                                    "type": "array",
                                    "items": {"type": "string"}
                                },
                                "retry_count": {
                                    "type": "integer",
                                    "default": 3
                                },
                                "retry_delay_ms": {
                                    "type": "integer",
                                    "default": 1000
                                }
                            },
                            "required": ["action"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "clear_input"
                                },
                                "select_all": {
                                    "type": "boolean",
                                    "default": true
                                }
                            },
                            "required": ["action"]
                        }
                    ]
                }),
            },
        }
    }

    async fn navigate(&self, direction: &str, count: u32) -> Result<()> {
        let key = match direction.to_lowercase().as_str() {
            "up" => "up",
            "down" => "down",
            "left" => "left",
            "right" => "right",
            "home" => "home",
            "end" => "end",
            "pageup" | "page_up" => "pageup",
            "pagedown" | "page_down" => "pagedown",
            _ => {
                return Err(ToolError::Mcp(format!(
                    "Unknown navigation direction: {direction}"
                )));
            }
        };

        for _ in 0..count {
            self.keyboard
                .execute(json!({
                    "action": "key",
                    "key": key
                }))
                .await?;

            // Small delay between navigation actions
            sleep(Duration::from_millis(50)).await;
        }

        Ok(())
    }

    async fn type_and_verify(
        &self,
        text: &str,
        verify_delay_ms: u64,
        expected_content: Option<&str>,
    ) -> Result<Value> {
        // Type the text
        self.keyboard
            .execute(json!({
                "action": "text",
                "text": text
            }))
            .await?;

        // Wait for UI to update
        sleep(Duration::from_millis(verify_delay_ms)).await;

        // Capture screenshot for verification
        let screenshot_result = self
            .screenshot
            .execute(json!({
                "format": "text",
                "include_colors": false
            }))
            .await?;

        let content = screenshot_result["content"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Failed to get screenshot content".to_string()))?;

        // Verify if expected
        let verification_passed = if let Some(expected) = expected_content {
            content.contains(expected)
        } else {
            true
        };

        Ok(json!({
            "success": true,
            "typed_text": text,
            "verification_passed": verification_passed,
            "captured_content": content
        }))
    }

    async fn shortcut_and_capture(
        &self,
        shortcut: &str,
        modifiers: &[String],
        capture_delay_ms: u64,
    ) -> Result<Value> {
        // Send the shortcut
        self.keyboard
            .execute(json!({
                "action": "shortcut",
                "shortcut": shortcut,
                "modifiers": modifiers
            }))
            .await?;

        // Wait for UI to update
        sleep(Duration::from_millis(capture_delay_ms)).await;

        // Capture the result
        let screenshot_result = self
            .screenshot
            .execute(json!({
                "format": "text",
                "include_colors": true
            }))
            .await?;

        Ok(json!({
            "success": true,
            "shortcut": format!("{:?}+{}", modifiers, shortcut),
            "captured_content": screenshot_result["content"]
        }))
    }

    async fn verify_state(
        &self,
        expected_content: Option<&str>,
        contains_text: Option<&[String]>,
        retry_count: u32,
        retry_delay_ms: u64,
    ) -> Result<Value> {
        let mut last_content = String::new();
        let mut attempt = 0;

        while attempt <= retry_count {
            // Capture current state
            let screenshot_result = self
                .screenshot
                .execute(json!({
                    "format": "text",
                    "include_colors": false
                }))
                .await?;

            last_content = screenshot_result["content"]
                .as_str()
                .ok_or_else(|| ToolError::Mcp("Failed to get screenshot content".to_string()))?
                .to_string();

            // Check expected content
            let mut all_checks_passed = true;

            if let Some(expected) = expected_content
                && !last_content.contains(expected)
            {
                all_checks_passed = false;
            }

            if let Some(texts) = contains_text {
                for text in texts {
                    if !last_content.contains(text) {
                        all_checks_passed = false;
                        break;
                    }
                }
            }

            if all_checks_passed {
                return Ok(json!({
                    "success": true,
                    "verification_passed": true,
                    "attempts": attempt + 1,
                    "content": last_content
                }));
            }

            attempt += 1;
            if attempt <= retry_count {
                sleep(Duration::from_millis(retry_delay_ms)).await;
            }
        }

        Ok(json!({
            "success": true,
            "verification_passed": false,
            "attempts": retry_count + 1,
            "content": last_content,
            "message": "State verification failed after all retries"
        }))
    }

    async fn clear_input(&self, select_all: bool) -> Result<()> {
        if select_all {
            // Select all with Ctrl+A
            self.keyboard
                .execute(json!({
                    "action": "shortcut",
                    "shortcut": "a",
                    "modifiers": ["ctrl"]
                }))
                .await?;

            sleep(Duration::from_millis(100)).await;
        }

        // Delete/backspace
        self.keyboard
            .execute(json!({
                "action": "key",
                "key": "delete"
            }))
            .await?;

        Ok(())
    }
}

#[async_trait]
impl Tool for TuiInteractionKit {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action: InteractionAction = serde_json::from_value(args)
            .map_err(|e| ToolError::Mcp(format!("Invalid parameters: {e}")))?;

        match action {
            InteractionAction::Navigate { direction, count } => {
                self.navigate(&direction, count).await?;
                Ok(json!({
                    "success": true,
                    "message": format!("Navigated {} {} times", direction, count)
                }))
            }
            InteractionAction::TypeAndVerify {
                text,
                verify_delay_ms,
                expected_content,
            } => Ok(self
                .type_and_verify(&text, verify_delay_ms, expected_content.as_deref())
                .await?),
            InteractionAction::ShortcutAndCapture {
                shortcut,
                modifiers,
                capture_delay_ms,
            } => Ok(self
                .shortcut_and_capture(&shortcut, &modifiers, capture_delay_ms)
                .await?),
            InteractionAction::VerifyState {
                expected_content,
                contains_text,
                retry_count,
                retry_delay_ms,
            } => {
                let texts_slice = contains_text.as_deref().unwrap_or(&[]);
                Ok(self
                    .verify_state(
                        expected_content.as_deref(),
                        Some(texts_slice),
                        retry_count,
                        retry_delay_ms,
                    )
                    .await?)
            }
            InteractionAction::ClearInput { select_all } => {
                self.clear_input(select_all).await?;
                Ok(json!({
                    "success": true,
                    "message": "Input cleared"
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let kit = TuiInteractionKit::new();
        let schema = kit.schema();
        assert_eq!(schema.name, "tui_interaction");
    }
}
