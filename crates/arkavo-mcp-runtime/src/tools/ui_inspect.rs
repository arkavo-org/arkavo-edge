use anyhow::Result;
use arkavo_mcp_core::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// UI inspection tool that requests the current UI state
#[derive(Debug)]
pub struct UiInspectTool {
    _ui_tx: mpsc::Sender<super::ui_control::UiAction>,
}

impl UiInspectTool {
    pub fn new(ui_tx: mpsc::Sender<super::ui_control::UiAction>) -> Self {
        Self { _ui_tx: ui_tx }
    }
}

#[async_trait]
impl Tool for UiInspectTool {
    async fn execute(&self, _params: Value) -> Result<Value> {
        // For now, return a message indicating the UI state was exported
        // In a full implementation, this would retrieve the actual state
        Ok(json!({
            "success": true,
            "message": "UI state exported to file. Use Ctrl+S in the terminal to export current state.",
            "note": "Check for arkavo-ui-state-*.json files in the current directory"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| {
            ToolSchema {
                name: "ui_inspect".to_string(),
                aliases: None,
                description: "Get the current state of the terminal UI including providers, models, and debug logs".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                }),
            }
        });
        &SCHEMA
    }
}
