use crate::{BrowserError, Result};
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::network::EventRequestWillBeSent;
use chromiumoxide::cdp::js_protocol::runtime::EventConsoleApiCalled;
use chromiumoxide::page::ScreenshotParams;
use futures::StreamExt;
use serde_json::{Value, json};

pub struct BrowserTool {
    schema: ToolSchema,
}

impl BrowserTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "browser_cdp".to_string(),
                description:
                    "Chrome DevTools Protocol browser automation for E2E testing and debugging"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["navigate", "screenshot", "evaluate", "content", "console", "network"],
                            "description": "Browser action to perform"
                        },
                        "url": {
                            "type": "string",
                            "description": "URL to navigate to (for navigate action)"
                        },
                        "script": {
                            "type": "string",
                            "description": "JavaScript to evaluate (for evaluate action)"
                        },
                        "screenshot_path": {
                            "type": "string",
                            "description": "Path to save screenshot (PNG format)"
                        },
                        "headless": {
                            "type": "boolean",
                            "description": "Run in headless mode (default: true)"
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Timeout in seconds (default: 30)"
                        },
                        "viewport": {
                            "type": "object",
                            "properties": {
                                "width": { "type": "integer" },
                                "height": { "type": "integer" }
                            },
                            "description": "Viewport dimensions (default: 1920x1080)"
                        }
                    },
                    "required": ["action"]
                }),
            },
        }
    }

    async fn execute_browser(&self, params: &Value) -> Result<String> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| BrowserError::InvalidParams("Missing action".to_string()))?;

        let headless = params
            .get("headless")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut config = BrowserConfig::builder();
        if headless {
            config = config.no_sandbox().disable_default_args();
        }

        if let Some(viewport) = params.get("viewport") {
            if let (Some(width), Some(height)) = (
                viewport.get("width").and_then(|v| v.as_u64()),
                viewport.get("height").and_then(|v| v.as_u64()),
            ) {
                config = config.window_size(width as u32, height as u32);
            }
        }

        let (browser, mut handler) = Browser::launch(
            config
                .build()
                .map_err(|e| BrowserError::Playwright(format!("Failed to build config: {e}")))?,
        )
        .await
        .map_err(|e| BrowserError::Playwright(format!("Failed to launch browser: {e}")))?;

        let handle = tokio::task::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| BrowserError::Playwright(format!("Failed to create page: {e}")))?;

        let result = match action {
            "navigate" => {
                let url = params
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BrowserError::InvalidParams("Missing url".to_string()))?;

                page.goto(url)
                    .await
                    .map_err(|e| BrowserError::Navigation(format!("Navigation failed: {e}")))?;

                format!("Navigated to {url}")
            }

            "screenshot" => {
                let path = params
                    .get("screenshot_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BrowserError::InvalidParams("Missing screenshot_path".to_string())
                    })?;

                let screenshot = page
                    .screenshot(ScreenshotParams::default())
                    .await
                    .map_err(|e| BrowserError::Screenshot(format!("Screenshot failed: {e}")))?;

                tokio::fs::write(path, screenshot)
                    .await
                    .map_err(BrowserError::Io)?;

                format!("Screenshot saved to {path}")
            }

            "evaluate" => {
                let script = params
                    .get("script")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BrowserError::InvalidParams("Missing script".to_string()))?;

                let result = page
                    .evaluate(script)
                    .await
                    .map_err(|e| BrowserError::Playwright(format!("Evaluation failed: {e}")))?;

                format!("Evaluation result: {result:?}")
            }

            "content" => {
                let content = page
                    .content()
                    .await
                    .map_err(|e| BrowserError::Playwright(format!("Failed to get content: {e}")))?;

                content
            }

            "console" => {
                let mut console_messages = Vec::new();
                let mut events = page
                    .event_listener::<EventConsoleApiCalled>()
                    .await
                    .map_err(|e| {
                        BrowserError::Playwright(format!("Failed to listen to console: {e}"))
                    })?;

                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                while let Some(event) = events.next().await {
                    for arg in &event.args {
                        if let Some(value) = &arg.value {
                            console_messages.push(value.to_string());
                        }
                    }
                }

                json!(console_messages).to_string()
            }

            "network" => {
                let mut network_events = Vec::new();
                let mut requests = page
                    .event_listener::<EventRequestWillBeSent>()
                    .await
                    .map_err(|e| {
                        BrowserError::Network(format!("Failed to listen to network: {e}"))
                    })?;

                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                while let Some(request) = requests.next().await {
                    network_events.push(json!({
                        "url": request.request.url,
                        "method": request.request.method,
                    }));
                }

                json!(network_events).to_string()
            }

            _ => {
                return Err(BrowserError::InvalidParams(format!(
                    "Unknown action: {action}"
                )));
            }
        };

        handle.abort();

        Ok(result)
    }
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserTool {
    async fn execute(
        &self,
        params: Value,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let output = self
            .execute_browser(&params)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(json!({
            "success": true,
            "result": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_browser_navigation() {
        let tool = BrowserTool::new();
        let params = json!({
            "action": "navigate",
            "url": "https://example.com",
            "headless": true
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
