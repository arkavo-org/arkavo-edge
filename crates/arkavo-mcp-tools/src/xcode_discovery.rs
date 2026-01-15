use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;

/// Xcode project information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XcodeProject {
    pub path: String,
    pub name: String,
    pub project_type: String,
}

/// Execute xcodebuild command and return output
async fn run_xcodebuild(args: &[&str], working_dir: Option<&str>) -> Result<String> {
    let mut cmd = Command::new("xcodebuild");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| ToolError::Execution(format!("Failed to run xcodebuild: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::Execution(format!("xcodebuild failed: {}", stderr)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Recursively find Xcode projects in a directory
async fn find_xcode_projects(start_path: &str, max_depth: u32) -> Vec<XcodeProject> {
    let mut projects = Vec::new();
    find_projects_recursive(start_path, 0, max_depth, &mut projects).await;
    projects
}

#[async_recursion::async_recursion]
async fn find_projects_recursive(
    path: &str,
    current_depth: u32,
    max_depth: u32,
    projects: &mut Vec<XcodeProject>,
) {
    if current_depth > max_depth {
        return;
    }

    let entries = match tokio::fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(_) => return,
    };

    let mut entries = entries;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let entry_path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        if file_name.starts_with('.') {
            continue;
        }

        if file_name.ends_with(".xcworkspace") {
            projects.push(XcodeProject {
                path: entry_path.to_string_lossy().to_string(),
                name: file_name.trim_end_matches(".xcworkspace").to_string(),
                project_type: "workspace".to_string(),
            });
        } else if file_name.ends_with(".xcodeproj") {
            projects.push(XcodeProject {
                path: entry_path.to_string_lossy().to_string(),
                name: file_name.trim_end_matches(".xcodeproj").to_string(),
                project_type: "project".to_string(),
            });
        } else if entry_path.is_dir() {
            find_projects_recursive(
                &entry_path.to_string_lossy(),
                current_depth + 1,
                max_depth,
                projects,
            )
            .await;
        }
    }
}

// ============================================================================
// XcodeDiscoverTool - Discover Xcode projects in a directory
// ============================================================================

pub struct XcodeDiscoverTool {
    schema: ToolSchema,
}

impl XcodeDiscoverTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_discover".to_string(),
                aliases: Some(vec!["discover_projects".to_string(), "find_xcode".to_string()]),
                description: "Discover Xcode projects (.xcodeproj) and workspaces (.xcworkspace) in a directory tree.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "start_path": {
                            "type": "string",
                            "description": "Directory to start searching from (default: current directory)"
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum directory depth to search (default: 5)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for XcodeDiscoverTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeDiscoverTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let start_path = params
            .get("start_path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let max_depth = params
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as u32;

        let projects = find_xcode_projects(start_path, max_depth).await;

        let workspaces: Vec<_> = projects
            .iter()
            .filter(|p| p.project_type == "workspace")
            .collect();
        let xcodeprojs: Vec<_> = projects
            .iter()
            .filter(|p| p.project_type == "project")
            .collect();

        Ok(json!({
            "success": true,
            "count": projects.len(),
            "workspaces": workspaces,
            "projects": xcodeprojs,
            "all": projects
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// XcodeListSchemesTool - List schemes in a project/workspace
// ============================================================================

pub struct XcodeListSchemesTool {
    schema: ToolSchema,
}

impl XcodeListSchemesTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_list_schemes".to_string(),
                aliases: Some(vec!["list_schemes".to_string(), "schemes".to_string()]),
                description: "List available schemes in an Xcode project or workspace.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for XcodeListSchemesTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeListSchemesTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let mut args = vec!["-list", "-json"];

        let project_path = params.get("project_path").and_then(|v| v.as_str());
        let workspace_path = params.get("workspace_path").and_then(|v| v.as_str());

        let path_arg: String;
        if let Some(workspace) = workspace_path {
            path_arg = format!("-workspace {}", workspace);
            args.push("-workspace");
            args.push(workspace);
        } else if let Some(project) = project_path {
            path_arg = format!("-project {}", project);
            args.push("-project");
            args.push(project);
        } else {
            return Err(ToolError::InvalidParams(
                "Either 'project_path' or 'workspace_path' is required".to_string(),
            ));
        }

        let _ = path_arg;
        let output = run_xcodebuild(&args, None).await?;

        // Parse JSON output
        let json_output: Value = serde_json::from_str(&output).map_err(|e| {
            ToolError::Execution(format!("Failed to parse xcodebuild output: {}", e))
        })?;

        let schemes: Vec<String> = json_output
            .get("project")
            .or_else(|| json_output.get("workspace"))
            .and_then(|p| p.get("schemes"))
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Ok(json!({
            "success": true,
            "count": schemes.len(),
            "schemes": schemes
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// XcodeShowSettingsTool - Show build settings
// ============================================================================

pub struct XcodeShowSettingsTool {
    schema: ToolSchema,
}

impl XcodeShowSettingsTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_show_settings".to_string(),
                aliases: Some(vec!["build_settings".to_string()]),
                description: "Show Xcode build settings for a scheme.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace"
                        },
                        "scheme": {
                            "type": "string",
                            "description": "Scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "description": "Build configuration (Debug/Release)"
                        },
                        "settings": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Specific settings to retrieve (returns all if not specified)"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for XcodeShowSettingsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeShowSettingsTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params.get("scheme").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'scheme' is required".to_string())
        })?;

        let mut args = vec!["-showBuildSettings", "-scheme", scheme];

        if let Some(workspace) = params.get("workspace_path").and_then(|v| v.as_str()) {
            args.push("-workspace");
            args.push(workspace);
        } else if let Some(project) = params.get("project_path").and_then(|v| v.as_str()) {
            args.push("-project");
            args.push(project);
        }

        let config;
        if let Some(configuration) = params.get("configuration").and_then(|v| v.as_str()) {
            config = configuration.to_string();
            args.push("-configuration");
            args.push(&config);
        }

        let output = run_xcodebuild(&args, None).await?;

        // Parse build settings output
        let mut settings = serde_json::Map::new();
        for line in output.lines() {
            let trimmed = line.trim();
            if let Some((key, value)) = trimmed.split_once(" = ") {
                settings.insert(key.trim().to_string(), json!(value.trim()));
            }
        }

        // Filter specific settings if requested
        let filtered_settings: serde_json::Map<String, Value> =
            if let Some(requested) = params.get("settings").and_then(|v| v.as_array()) {
                let requested: Vec<String> = requested
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                settings
                    .into_iter()
                    .filter(|(k, _)| requested.contains(k))
                    .collect()
            } else {
                settings
            };

        Ok(json!({
            "success": true,
            "scheme": scheme,
            "settings": filtered_settings
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// XcodeGetBundleIdTool - Get bundle identifier
// ============================================================================

pub struct XcodeGetBundleIdTool {
    schema: ToolSchema,
}

impl XcodeGetBundleIdTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_get_bundle_id".to_string(),
                aliases: Some(vec!["bundle_id".to_string()]),
                description: "Get the bundle identifier for an Xcode scheme.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace"
                        },
                        "scheme": {
                            "type": "string",
                            "description": "Scheme name"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for XcodeGetBundleIdTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeGetBundleIdTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params.get("scheme").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'scheme' is required".to_string())
        })?;

        let mut args = vec!["-showBuildSettings", "-scheme", scheme];

        if let Some(workspace) = params.get("workspace_path").and_then(|v| v.as_str()) {
            args.push("-workspace");
            args.push(workspace);
        } else if let Some(project) = params.get("project_path").and_then(|v| v.as_str()) {
            args.push("-project");
            args.push(project);
        }

        let output = run_xcodebuild(&args, None).await?;

        // Find PRODUCT_BUNDLE_IDENTIFIER in output
        let bundle_id = output
            .lines()
            .find(|line| line.contains("PRODUCT_BUNDLE_IDENTIFIER"))
            .and_then(|line| line.split(" = ").nth(1))
            .map(|s| s.trim().to_string());

        match bundle_id {
            Some(id) => Ok(json!({
                "success": true,
                "scheme": scheme,
                "bundle_id": id
            })),
            None => Ok(json!({
                "success": false,
                "error": "Could not find PRODUCT_BUNDLE_IDENTIFIER in build settings"
            })),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xcode_discover_tool_schema() {
        let tool = XcodeDiscoverTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_discover");
        assert!(schema
            .aliases
            .as_ref()
            .unwrap()
            .contains(&"find_xcode".to_string()));
    }

    #[test]
    fn test_xcode_list_schemes_tool_schema() {
        let tool = XcodeListSchemesTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_list_schemes");
        assert!(schema.aliases.as_ref().unwrap().contains(&"schemes".to_string()));
    }

    #[test]
    fn test_xcode_show_settings_tool_schema() {
        let tool = XcodeShowSettingsTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_show_settings");
    }

    #[test]
    fn test_xcode_get_bundle_id_tool_schema() {
        let tool = XcodeGetBundleIdTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_get_bundle_id");
        assert!(schema
            .aliases
            .as_ref()
            .unwrap()
            .contains(&"bundle_id".to_string()));
    }
}
