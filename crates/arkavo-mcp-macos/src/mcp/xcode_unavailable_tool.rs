use super::server::{Tool, ToolSchema};
use super::xcode_version::XcodeVersion;
use crate::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

/// A placeholder tool that returns helpful error messages when Xcode is not available
pub struct XcodeUnavailableTool {
    schema: ToolSchema,
}

impl XcodeUnavailableTool {
    pub fn new(name: String, description: String, parameters: Value) -> Self {
        Self {
            schema: ToolSchema {
                name,
                description,
                parameters,
            },
        }
    }
}

#[async_trait]
impl Tool for XcodeUnavailableTool {
    async fn execute(&self, _params: Value) -> Result<Value> {
        Ok(json!({
            "error": {
                "code": "XCODE_REQUIRED",
                "message": format!("The '{}' tool requires Xcode Command Line Tools", self.schema.name),
                "details": "This tool is used for iOS simulator testing and automation",
                "suggestion": XcodeVersion::require_xcode_message(),
                "alternative": "You can still use non-iOS testing features of Arkavo Edge"
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
