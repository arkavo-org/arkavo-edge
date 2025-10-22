use super::server::Tool;
use crate::{Result, TestError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Command;

pub struct TuiScreenshotKit {
    schema: ToolSchema,
}

#[derive(Debug, Serialize, Deserialize)]
struct ScreenshotParams {
    #[serde(default = "default_format")]
    format: ScreenshotFormat,
    #[serde(default = "default_include_colors")]
    include_colors: bool,
    #[serde(default)]
    window_title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ScreenshotFormat {
    Text,
    Ansi,
    Image,
}

fn default_format() -> ScreenshotFormat {
    ScreenshotFormat::Text
}

fn default_include_colors() -> bool {
    true
}

impl Default for TuiScreenshotKit {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiScreenshotKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "tui_screenshot".to_string(),
                description: "Capture TUI application screen state".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "format": {
                            "type": "string",
                            "enum": ["text", "ansi", "image"],
                            "default": "text",
                            "description": "Output format: text (plain), ansi (with colors), or image (PNG base64)"
                        },
                        "include_colors": {
                            "type": "boolean",
                            "default": true,
                            "description": "Include ANSI color codes in text format"
                        },
                        "window_title": {
                            "type": "string",
                            "description": "Optional window title to capture (captures active window if not specified)"
                        }
                    }
                }),
            },
        }
    }

    async fn capture_terminal_output(&self, params: &ScreenshotParams) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            match params.format {
                ScreenshotFormat::Text | ScreenshotFormat::Ansi => {
                    self.capture_terminal_text_macos(params).await
                }
                ScreenshotFormat::Image => self.capture_terminal_image_macos(params).await,
            }
        }

        #[cfg(target_os = "linux")]
        {
            match params.format {
                ScreenshotFormat::Text | ScreenshotFormat::Ansi => {
                    self.capture_terminal_text_linux(params).await
                }
                ScreenshotFormat::Image => self.capture_terminal_image_linux(params).await,
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(TestError::Mcp(
                "Terminal screenshot not supported on this platform".to_string(),
            ))
        }
    }

    #[cfg(target_os = "macos")]
    async fn capture_terminal_text_macos(&self, params: &ScreenshotParams) -> Result<String> {
        // Use AppleScript to get terminal content
        let script = if let Some(title) = &params.window_title {
            format!(
                r#"
                tell application "Terminal"
                    set targetWindow to null
                    repeat with w in windows
                        if name of w contains "{title}" then
                            set targetWindow to w
                            exit repeat
                        end if
                    end repeat
                    
                    if targetWindow is not null then
                        set currentTab to selected tab of targetWindow
                        return contents of currentTab
                    else
                        error "Window not found"
                    end if
                end tell
                "#
            )
        } else {
            r#"
            tell application "Terminal"
                if (count of windows) > 0 then
                    return contents of selected tab of front window
                else
                    error "No terminal windows open"
                end if
            end tell
            "#
            .to_string()
        };

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute osascript: {e}")))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to capture terminal: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let content = String::from_utf8_lossy(&output.stdout).to_string();

        // Process based on format
        match params.format {
            ScreenshotFormat::Text => {
                if params.include_colors {
                    Ok(content)
                } else {
                    // Strip ANSI escape codes
                    Ok(self.strip_ansi_codes(&content))
                }
            }
            ScreenshotFormat::Ansi => Ok(content),
            _ => unreachable!(),
        }
    }

    #[cfg(target_os = "linux")]
    async fn capture_terminal_text_linux(&self, params: &ScreenshotParams) -> Result<String> {
        // Try to use xdotool and xclip to get terminal content
        // First, find the terminal window
        let window_id = if let Some(title) = &params.window_title {
            let output = Command::new("xdotool")
                .args(["search", "--name", title])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to find window: {}", e)))?;

            if !output.status.success() {
                return Err(TestError::Mcp("Failed to find terminal window".to_string()));
            }

            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            // Get the active window
            let output = Command::new("xdotool")
                .args(["getactivewindow"])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to get active window: {}", e)))?;

            if !output.status.success() {
                return Err(TestError::Mcp("Failed to get active window".to_string()));
            }

            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };

        // Select all and copy
        Command::new("xdotool")
            .args([
                "windowfocus",
                &window_id,
                "key",
                "ctrl+shift+a",
                "ctrl+shift+c",
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to copy terminal content: {}", e)))?;

        // Get clipboard content
        let output = Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get clipboard: {}", e)))?;

        let content = String::from_utf8_lossy(&output.stdout).to_string();

        match params.format {
            ScreenshotFormat::Text => {
                if params.include_colors {
                    Ok(content)
                } else {
                    Ok(self.strip_ansi_codes(&content))
                }
            }
            ScreenshotFormat::Ansi => Ok(content),
            _ => unreachable!(),
        }
    }

    #[cfg(target_os = "macos")]
    async fn capture_terminal_image_macos(&self, params: &ScreenshotParams) -> Result<String> {
        use base64::{engine::general_purpose, Engine as _};
        use std::fs;

        let temp_file = format!("/tmp/tui_screenshot_{}.png", std::process::id());

        // Use screencapture to capture window
        let mut cmd = Command::new("screencapture");
        cmd.args(["-x", "-o"]); // No sound, no shadow

        if let Some(_title) = &params.window_title {
            // Try to find window by title
            cmd.args(["-l", &self.find_window_id_by_title(_title)?]);
        } else {
            // Interactive window selection
            cmd.arg("-W");
        }

        cmd.arg(&temp_file);

        let output = cmd
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to capture screenshot: {e}")))?;

        if !output.status.success() {
            return Err(TestError::Mcp("Failed to capture screenshot".to_string()));
        }

        // Read and encode the image
        let image_data = fs::read(&temp_file)
            .map_err(|e| TestError::Mcp(format!("Failed to read screenshot: {e}")))?;

        // Clean up
        let _ = fs::remove_file(&temp_file);

        // Return base64 encoded image
        Ok(general_purpose::STANDARD.encode(&image_data))
    }

    #[cfg(target_os = "linux")]
    async fn capture_terminal_image_linux(&self, params: &ScreenshotParams) -> Result<String> {
        use base64::{engine::general_purpose, Engine as _};
        use std::fs;

        let temp_file = format!("/tmp/tui_screenshot_{}.png", std::process::id());

        // Use import (ImageMagick) or scrot
        let window_id = if let Some(title) = &params.window_title {
            let output = Command::new("xdotool")
                .args(["search", "--name", title])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to find window: {}", e)))?;

            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            let output = Command::new("xdotool")
                .args(["getactivewindow"])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to get active window: {}", e)))?;

            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };

        // Capture with import
        let output = Command::new("import")
            .args(["-window", &window_id, &temp_file])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to capture screenshot: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp("Failed to capture screenshot".to_string()));
        }

        // Read and encode the image
        let image_data = fs::read(&temp_file)
            .map_err(|e| TestError::Mcp(format!("Failed to read screenshot: {}", e)))?;

        // Clean up
        let _ = fs::remove_file(&temp_file);

        // Return base64 encoded image
        Ok(general_purpose::STANDARD.encode(&image_data))
    }

    fn strip_ansi_codes(&self, text: &str) -> String {
        // Simple ANSI escape code stripper
        let re = regex::Regex::new(r"\x1B\[[0-9;]*[a-zA-Z]").unwrap();
        re.replace_all(text, "").to_string()
    }

    #[cfg(target_os = "macos")]
    fn find_window_id_by_title(&self, _title: &str) -> Result<String> {
        // Use Quartz Window Services to find window
        // For now, return error as this requires more complex implementation
        Err(TestError::Mcp(
            "Window selection by title not yet implemented".to_string(),
        ))
    }
}

#[async_trait]
impl Tool for TuiScreenshotKit {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let params: ScreenshotParams = if args.is_null() {
            ScreenshotParams {
                format: default_format(),
                include_colors: default_include_colors(),
                window_title: None,
            }
        } else {
            serde_json::from_value(args)
                .map_err(|e| TestError::Mcp(format!("Invalid parameters: {e}")))?
        };

        let content = self.capture_terminal_output(&params).await?;

        Ok(json!({
            "success": true,
            "format": params.format,
            "content": content,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let kit = TuiScreenshotKit::new();
        let schema = kit.schema();
        assert_eq!(schema.name, "tui_screenshot");
    }

    #[test]
    fn test_strip_ansi_codes() {
        let kit = TuiScreenshotKit::new();
        let text_with_ansi = "\x1B[31mRed Text\x1B[0m Normal Text";
        let stripped = kit.strip_ansi_codes(text_with_ansi);
        assert_eq!(stripped, "Red Text Normal Text");
    }
}
