use super::server::{Tool, ToolSchema};
use super::templates;
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct TemplateDiagnosticsKit {
    schema: ToolSchema,
}

impl TemplateDiagnosticsKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "template_diagnostics".to_string(),
                description: "Diagnose template location and version issues. Shows where templates are being loaded from and their content.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        }
    }
}

impl Default for TemplateDiagnosticsKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TemplateDiagnosticsKit {
    async fn execute(&self, _params: Value) -> Result<Value> {
        // Check embedded templates
        let embedded_templates = serde_json::json!({
            "basic_swift": {
                "is_embedded": true,
                "size_bytes": templates::ARKAVO_TEST_RUNNER_SWIFT.len(),
                "has_json_value": templates::ARKAVO_TEST_RUNNER_SWIFT.contains("enum JSONValue: Codable"),
                "has_old_string_any": templates::ARKAVO_TEST_RUNNER_SWIFT.contains("let result: [String: Any]?"),
                "line_count": templates::ARKAVO_TEST_RUNNER_SWIFT.lines().count(),
            },
            "enhanced_swift": {
                "is_embedded": true,
                "size_bytes": templates::ARKAVO_TEST_RUNNER_ENHANCED_SWIFT.len(),
                "has_json_value": templates::ARKAVO_TEST_RUNNER_ENHANCED_SWIFT.contains("enum JSONValue: Codable"),
                "has_old_string_any": templates::ARKAVO_TEST_RUNNER_ENHANCED_SWIFT.contains("let result: [String: Any]?"),
                "line_count": templates::ARKAVO_TEST_RUNNER_ENHANCED_SWIFT.lines().count(),
            },
            "info_plist": {
                "is_embedded": true,
                "size_bytes": templates::INFO_PLIST.len(),
                "has_bundle_id": templates::INFO_PLIST.contains("CFBundleIdentifier"),
            }
        });
        
        let diagnostics = serde_json::json!({
            "status": "Templates are now embedded in the binary at compile time",
            "binary_info": {
                "executable_path": std::env::current_exe().ok(),
                "current_dir": std::env::current_dir().ok(),
                "cargo_manifest_dir": env!("CARGO_MANIFEST_DIR"),
            },
            "embedded_templates": embedded_templates,
            "filesystem_templates": {
                "status": "No longer used - templates are compiled into the binary",
                "reason": "Ensures template consistency and eliminates runtime filesystem dependencies"
            }
        });
        
        Ok(diagnostics)
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}