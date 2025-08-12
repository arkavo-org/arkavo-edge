use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

pub struct TuiKeyboardKit {
    retry_attempts: u32,
    retry_delay: Duration,
    schema: ToolSchema,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action")]
enum KeyboardAction {
    #[serde(rename = "key")]
    Key {
        key: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "shortcut")]
    Shortcut {
        shortcut: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
}

impl Default for TuiKeyboardKit {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiKeyboardKit {
    pub fn new() -> Self {
        Self {
            retry_attempts: 3,
            retry_delay: Duration::from_millis(500),
            schema: ToolSchema {
                name: "tui_keyboard".to_string(),
                description: "Send keyboard input to TUI applications".to_string(),
                parameters: json!({
                    "type": "object",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "key"
                                },
                                "key": {
                                    "type": "string",
                                    "description": "Key name (enter, escape, up, down, tab, etc.)"
                                },
                                "modifiers": {
                                    "type": "array",
                                    "items": {
                                        "type": "string",
                                        "enum": ["ctrl", "alt", "shift", "cmd"]
                                    },
                                    "default": []
                                }
                            },
                            "required": ["action", "key"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "text"
                                },
                                "text": {
                                    "type": "string",
                                    "description": "Text to type"
                                }
                            },
                            "required": ["action", "text"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "shortcut"
                                },
                                "shortcut": {
                                    "type": "string",
                                    "description": "Shortcut key combination (e.g., 'c' for Ctrl+C)"
                                },
                                "modifiers": {
                                    "type": "array",
                                    "items": {
                                        "type": "string",
                                        "enum": ["ctrl", "alt", "shift", "cmd"]
                                    },
                                    "default": ["ctrl"]
                                }
                            },
                            "required": ["action", "shortcut"]
                        }
                    ]
                }),
            },
        }
    }

    async fn send_key_sequence(&self, sequence: &str) -> Result<()> {
        // For macOS, we can use osascript to send keystrokes
        // For Linux, we'd use xdotool or similar
        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"tell application "System Events" to keystroke "{}""#,
                sequence.replace('"', r#"\""#)
            );

            let mut attempt = 0;
            while attempt < self.retry_attempts {
                let result = Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .output()
                    .map_err(|e| ToolError::Mcp(format!("Failed to execute osascript: {e}")))?;

                if result.status.success() {
                    return Ok(());
                }

                attempt += 1;
                if attempt < self.retry_attempts {
                    sleep(self.retry_delay).await;
                }
            }

            Err(ToolError::Mcp(format!(
                "Failed to send key sequence after {} attempts",
                self.retry_attempts
            )))
        }

        #[cfg(not(target_os = "macos"))]
        {
            // For Linux, use xdotool if available
            let mut attempt = 0;
            while attempt < self.retry_attempts {
                let result = Command::new("xdotool").arg("type").arg(sequence).output();

                match result {
                    Ok(output) if output.status.success() => return Ok(()),
                    Ok(_) => {}
                    Err(e) => return Err(ToolError::Mcp(format!("xdotool not available: {}", e))),
                }

                attempt += 1;
                if attempt < self.retry_attempts {
                    sleep(self.retry_delay).await;
                }
            }

            Err(ToolError::Mcp(format!(
                "Failed to send key sequence after {} attempts",
                self.retry_attempts
            )))
        }
    }

    fn send_key_with_modifiers(&self, key: &str, modifiers: &[String]) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let modifier_str = modifiers
                .iter()
                .map(|m| match m.as_str() {
                    "ctrl" | "control" => "control down",
                    "alt" | "option" => "option down",
                    "shift" => "shift down",
                    "cmd" | "command" => "command down",
                    _ => "",
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ");

            let script = if modifier_str.is_empty() {
                format!(
                    r#"tell application "System Events" to key code {}"#,
                    self.key_to_code(key)?
                )
            } else {
                format!(
                    r#"tell application "System Events" to key code {} using {{{}}}"#,
                    self.key_to_code(key)?,
                    modifier_str
                )
            };

            let result = Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .map_err(|e| ToolError::Mcp(format!("Failed to execute osascript: {e}")))?;

            if !result.status.success() {
                return Err(ToolError::Mcp(format!(
                    "Failed to send key with modifiers: {}",
                    String::from_utf8_lossy(&result.stderr)
                )));
            }

            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            // For Linux, use xdotool
            let mut args = vec!["key".to_string()];

            let key_combo = if modifiers.is_empty() {
                key.to_string()
            } else {
                format!("{}+{}", modifiers.join("+"), key)
            };

            args.push(key_combo);

            let result = Command::new("xdotool")
                .args(&args)
                .output()
                .map_err(|e| ToolError::Mcp(format!("Failed to execute xdotool: {}", e)))?;

            if !result.status.success() {
                return Err(ToolError::Mcp(format!(
                    "Failed to send key with modifiers: {}",
                    String::from_utf8_lossy(&result.stderr)
                )));
            }

            Ok(())
        }
    }

    #[cfg(target_os = "macos")]
    fn key_to_code(&self, key: &str) -> Result<String> {
        // Map common keys to macOS key codes
        let code = match key.to_lowercase().as_str() {
            "enter" | "return" => "36",
            "tab" => "48",
            "space" => "49",
            "delete" | "backspace" => "51",
            "escape" | "esc" => "53",
            "up" => "126",
            "down" => "125",
            "left" => "123",
            "right" => "124",
            "a" => "0",
            "b" => "11",
            "c" => "8",
            "d" => "2",
            "e" => "14",
            "f" => "3",
            "g" => "5",
            "h" => "4",
            "i" => "34",
            "j" => "38",
            "k" => "40",
            "l" => "37",
            "m" => "46",
            "n" => "45",
            "o" => "31",
            "p" => "35",
            "q" => "12",
            "r" => "15",
            "s" => "1",
            "t" => "17",
            "u" => "32",
            "v" => "9",
            "w" => "13",
            "x" => "7",
            "y" => "16",
            "z" => "6",
            _ => return Err(ToolError::Mcp(format!("Unknown key: {key}"))),
        };

        Ok(code.to_string())
    }
}

#[async_trait]
impl Tool for TuiKeyboardKit {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action: KeyboardAction = serde_json::from_value(args)
            .map_err(|e| ToolError::Mcp(format!("Invalid parameters: {e}")))?;

        match action {
            KeyboardAction::Key { key, modifiers } => {
                if modifiers.is_empty() {
                    // Special handling for common navigation keys
                    match key.to_lowercase().as_str() {
                        "enter" | "return" | "tab" | "escape" | "esc" | "up" | "down" | "left"
                        | "right" => {
                            self.send_key_with_modifiers(&key, &[])?;
                        }
                        _ => {
                            // For single characters, use text typing
                            if key.len() == 1 {
                                self.send_key_sequence(&key).await?;
                            } else {
                                self.send_key_with_modifiers(&key, &[])?;
                            }
                        }
                    }
                } else {
                    self.send_key_with_modifiers(&key, &modifiers)?;
                }

                Ok(json!({
                    "success": true,
                    "message": format!("Sent key: {} with modifiers: {:?}", key, modifiers)
                }))
            }
            KeyboardAction::Text { text } => {
                self.send_key_sequence(&text).await?;

                Ok(json!({
                    "success": true,
                    "message": format!("Typed text: {}", text)
                }))
            }
            KeyboardAction::Shortcut {
                shortcut,
                modifiers,
            } => {
                let mods = if modifiers.is_empty() {
                    vec!["ctrl".to_string()]
                } else {
                    modifiers
                };

                self.send_key_with_modifiers(&shortcut, &mods)?;

                Ok(json!({
                    "success": true,
                    "message": format!("Sent shortcut: {:?}+{}", mods, shortcut)
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_keyboard_kit_creation() {
        let kit = TuiKeyboardKit::new();
        assert_eq!(kit.retry_attempts, 3);
    }

    #[test]
    fn test_schema() {
        let kit = TuiKeyboardKit::new();
        let schema = kit.schema();
        assert_eq!(schema.name, "tui_keyboard");
    }
}
