use anyhow::Result;
use arkavo_mcp_core::{Tool, ToolSchema};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "action")]
pub enum UiAction {
    #[serde(rename = "show_debug")]
    ShowDebug,
    #[serde(rename = "show_chat")]
    ShowChat,
    #[serde(rename = "show_code")]
    ShowCode,
    #[serde(rename = "show_diff")]
    ShowDiff,
    #[serde(rename = "show_dataflow")]
    ShowDataflow,
    #[serde(rename = "focus_input")]
    FocusInput,
    #[serde(rename = "focus_scroll")]
    FocusScroll,
}

/// UI control tool that sends commands to the terminal UI
#[derive(Debug)]
pub struct UiControlTool {
    ui_tx: mpsc::Sender<UiAction>,
}

impl UiControlTool {
    pub fn new(ui_tx: mpsc::Sender<UiAction>) -> Self {
        Self { ui_tx }
    }
}

#[async_trait]
impl Tool for UiControlTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action: UiAction = serde_json::from_value(params)?;

        // Send the action to the UI
        self.ui_tx
            .send(action)
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send UI command"))?;

        Ok(json!({
            "success": true,
            "message": "UI command sent"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "ui_control".to_string(),
            description: "Control the terminal UI view modes and focus".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "show_debug",
                            "show_chat",
                            "show_code",
                            "show_diff",
                            "show_dataflow",
                            "focus_input",
                            "focus_scroll"
                        ],
                        "description": "The UI action to perform"
                    }
                },
                "required": ["action"]
            }),
        });
        &SCHEMA
    }
}
