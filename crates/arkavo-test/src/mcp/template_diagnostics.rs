use super::server::{Tool, ToolSchema};
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

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
        let mut diagnostics = serde_json::json!({
            "binary_info": {
                "executable_path": std::env::current_exe().ok(),
                "current_dir": std::env::current_dir().ok(),
                "cargo_manifest_dir": env!("CARGO_MANIFEST_DIR"),
            },
            "template_search_paths": []
        });
        
        let search_paths = &mut Vec::new();
        
        // Method 1: Development location
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dev_template_dir = manifest_dir.join("templates").join("XCTestRunner");
        search_paths.push(check_template_at_path(&dev_template_dir, "development"));
        
        // Method 2: Relative to executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                // Installed location
                let installed_template_dir = exe_dir
                    .parent()
                    .map(|p| p.join("share/arkavo/templates/XCTestRunner"));
                    
                if let Some(dir) = installed_template_dir {
                    search_paths.push(check_template_at_path(&dir, "installed"));
                }
                
                // Relative to binary
                let relative_template_dir = exe_dir.parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.parent())
                    .map(|p| p.join("crates/arkavo-test/templates/XCTestRunner"));
                    
                if let Some(dir) = relative_template_dir {
                    search_paths.push(check_template_at_path(&dir, "relative_to_binary"));
                }
            }
        }
        
        // Method 3: Current directory
        let cwd = std::env::current_dir().unwrap_or_default();
        let cwd_template_dir = cwd.join("crates/arkavo-test/templates/XCTestRunner");
        search_paths.push(check_template_at_path(&cwd_template_dir, "current_directory"));
        
        diagnostics["template_search_paths"] = serde_json::json!(search_paths);
        
        // Find which one would be used
        let active_template = find_active_template();
        diagnostics["active_template"] = serde_json::json!(active_template);
        
        Ok(diagnostics)
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

fn check_template_at_path(path: &PathBuf, location_type: &str) -> serde_json::Value {
    let template_file = path.join("ArkavoTestRunner.swift.template");
    let exists = template_file.exists();
    
    let mut info = serde_json::json!({
        "location_type": location_type,
        "path": path.to_string_lossy(),
        "exists": exists,
        "template_file": template_file.to_string_lossy()
    });
    
    if exists {
        // Read and check template content
        if let Ok(content) = fs::read_to_string(&template_file) {
            let has_old_code = content.contains("let result: [String: Any]?");
            let has_json_value = content.contains("enum JSONValue: Codable");
            let line_count = content.lines().count();
            
            // Find the line with CommandResponse
            let command_response_line = content.lines()
                .enumerate()
                .find(|(_, line)| line.contains("struct CommandResponse"))
                .map(|(i, _)| i + 1);
                
            info["template_info"] = serde_json::json!({
                "has_old_string_any": has_old_code,
                "has_json_value": has_json_value,
                "line_count": line_count,
                "command_response_line": command_response_line,
                "is_updated": !has_old_code && has_json_value,
                "file_size": content.len()
            });
            
            // Get first few lines of CommandResponse struct
            if let Some(line_num) = command_response_line {
                let preview_lines: Vec<String> = content.lines()
                    .skip(line_num - 1)
                    .take(10)
                    .map(|s| s.to_string())
                    .collect();
                info["struct_preview"] = serde_json::json!(preview_lines);
            }
        }
    }
    
    info
}

fn find_active_template() -> serde_json::Value {
    // This mimics the logic in XCTestCompiler::find_template_dir()
    
    // Method 1: Development
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_template_dir = manifest_dir.join("templates").join("XCTestRunner");
    if dev_template_dir.exists() {
        return serde_json::json!({
            "found": true,
            "location": "development",
            "path": dev_template_dir.to_string_lossy()
        });
    }
    
    // Method 2: Relative to executable  
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let installed_template_dir = exe_dir
                .parent()
                .map(|p| p.join("share/arkavo/templates/XCTestRunner"));
                
            if let Some(dir) = installed_template_dir {
                if dir.exists() {
                    return serde_json::json!({
                        "found": true,
                        "location": "installed",
                        "path": dir.to_string_lossy()
                    });
                }
            }
            
            let relative_template_dir = exe_dir.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.join("crates/arkavo-test/templates/XCTestRunner"));
                
            if let Some(dir) = relative_template_dir {
                if dir.exists() {
                    return serde_json::json!({
                        "found": true,
                        "location": "relative_to_binary",
                        "path": dir.to_string_lossy()
                    });
                }
            }
        }
    }
    
    // Method 3: Current directory
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_template_dir = cwd.join("crates/arkavo-test/templates/XCTestRunner");
        if cwd_template_dir.exists() {
            return serde_json::json!({
                "found": true,
                "location": "current_directory", 
                "path": cwd_template_dir.to_string_lossy()
            });
        }
    }
    
    serde_json::json!({
        "found": false,
        "error": "No template directory found"
    })
}