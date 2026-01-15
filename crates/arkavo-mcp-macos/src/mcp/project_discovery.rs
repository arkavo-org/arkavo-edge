use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Run a shell command and return output
async fn run_command(cmd: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| TestError::Execution(format!("Failed to run {}: {}", cmd, e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TestError::Execution(format!("{} failed: {}", cmd, stderr)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ============================================================================
// DiscoverProjectsTool - Find Xcode projects and workspaces
// ============================================================================

pub struct DiscoverProjectsTool {
    schema: ToolSchema,
}

impl DiscoverProjectsTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "discover_projects".to_string(),
                aliases: Some(vec!["discover_projs".to_string(), "find_xcode_projects".to_string()]),
                description: "Scan a directory to find Xcode projects (.xcodeproj) and workspaces (.xcworkspace).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory to scan (default: current directory)"
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum recursion depth (default: 5)"
                        },
                        "include_pods": {
                            "type": "boolean",
                            "description": "Include Pods directory in scan (default: false)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for DiscoverProjectsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DiscoverProjectsTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let max_depth = params
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);

        let include_pods = params
            .get("include_pods")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Directories to skip
        let skip_dirs: HashSet<&str> = if include_pods {
            ["build", "DerivedData", ".git", "node_modules"]
                .iter()
                .copied()
                .collect()
        } else {
            ["build", "DerivedData", ".git", "node_modules", "Pods"]
                .iter()
                .copied()
                .collect()
        };

        let mut projects = Vec::new();
        let mut workspaces = Vec::new();

        // Use find command to discover projects
        let depth_str = max_depth.to_string();
        let find_output = Command::new("find")
            .arg(path)
            .arg("-maxdepth")
            .arg(&depth_str)
            .arg("-type")
            .arg("d")
            .arg("(")
            .arg("-name")
            .arg("*.xcodeproj")
            .arg("-o")
            .arg("-name")
            .arg("*.xcworkspace")
            .arg(")")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to run find: {}", e)))?;

        if find_output.status.success() {
            let output = String::from_utf8_lossy(&find_output.stdout);
            for line in output.lines() {
                if line.is_empty() {
                    continue;
                }

                // Check if path contains any skip directories
                let should_skip = skip_dirs
                    .iter()
                    .any(|skip| line.contains(&format!("/{}/", skip)));
                if should_skip {
                    continue;
                }

                // Filter out xcworkspace inside xcodeproj
                if line.contains(".xcodeproj/") && line.ends_with(".xcworkspace") {
                    continue;
                }

                if line.ends_with(".xcodeproj") {
                    projects.push(line.to_string());
                } else if line.ends_with(".xcworkspace") {
                    workspaces.push(line.to_string());
                }
            }
        }

        // Sort for consistent output
        projects.sort();
        workspaces.sort();

        Ok(json!({
            "success": true,
            "path": path,
            "projects": projects,
            "workspaces": workspaces,
            "project_count": projects.len(),
            "workspace_count": workspaces.len(),
            "total_found": projects.len() + workspaces.len()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// GetAppBundleIdTool - Extract bundle ID from any app bundle
// ============================================================================

pub struct GetAppBundleIdTool {
    schema: ToolSchema,
}

impl GetAppBundleIdTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "get_app_bundle_id".to_string(),
                aliases: Some(vec!["app_bundle_id".to_string(), "bundle_id".to_string()]),
                description: "Extract the bundle identifier from an app bundle (.app) for any Apple platform.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "app_path": {
                            "type": "string",
                            "description": "Path to the .app bundle"
                        }
                    },
                    "required": ["app_path"]
                }),
            },
        }
    }
}

impl Default for GetAppBundleIdTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GetAppBundleIdTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let app_path = params
            .get("app_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'app_path' is required".to_string()))?;

        if !Path::new(app_path).exists() {
            return Ok(json!({
                "success": false,
                "error": format!("App bundle not found: {}", app_path)
            }));
        }

        // Try different Info.plist locations for different platforms
        // macOS: AppName.app/Contents/Info.plist
        // iOS/watchOS/tvOS: AppName.app/Info.plist
        let info_plist_paths = [
            format!("{}/Contents/Info.plist", app_path), // macOS
            format!("{}/Info.plist", app_path),          // iOS/watchOS/tvOS/visionOS
        ];

        for plist_path in &info_plist_paths {
            if Path::new(plist_path).exists() {
                // Try PlistBuddy first
                if let Ok(output) = run_command(
                    "/usr/libexec/PlistBuddy",
                    &["-c", "Print :CFBundleIdentifier", plist_path],
                )
                .await
                {
                    let bundle_id = output.trim().to_string();
                    if !bundle_id.is_empty() {
                        return Ok(json!({
                            "success": true,
                            "app_path": app_path,
                            "bundle_id": bundle_id,
                            "info_plist": plist_path
                        }));
                    }
                }

                // Fallback to defaults command
                let info_path = plist_path.trim_end_matches(".plist");
                if let Ok(output) =
                    run_command("defaults", &["read", info_path, "CFBundleIdentifier"]).await
                {
                    let bundle_id = output.trim().to_string();
                    if !bundle_id.is_empty() {
                        return Ok(json!({
                            "success": true,
                            "app_path": app_path,
                            "bundle_id": bundle_id,
                            "info_plist": plist_path
                        }));
                    }
                }
            }
        }

        Ok(json!({
            "success": false,
            "app_path": app_path,
            "error": "Could not extract bundle ID - Info.plist not found or CFBundleIdentifier missing"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// ListSchemesTool - List available Xcode schemes
// ============================================================================

pub struct ListSchemesTool {
    schema: ToolSchema,
}

impl ListSchemesTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "list_schemes".to_string(),
                aliases: Some(vec!["xcode_schemes".to_string()]),
                description: "List available Xcode schemes for a project or workspace.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj file"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace file"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for ListSchemesTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListSchemesTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let mut args = vec!["-list"];

        let workspace;
        let project;
        let path_type;

        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            workspace = ws.to_string();
            args.push("-workspace");
            args.push(&workspace);
            path_type = "workspace";
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            project = proj.to_string();
            args.push("-project");
            args.push(&project);
            path_type = "project";
        } else {
            return Ok(json!({
                "success": false,
                "error": "Either 'project_path' or 'workspace_path' is required"
            }));
        }

        let output = Command::new("xcodebuild")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to run xcodebuild: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(json!({
                "success": false,
                "error": format!("xcodebuild -list failed: {}", stderr)
            }));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse schemes from output
        let mut schemes = Vec::new();
        let mut targets = Vec::new();
        let mut configurations = Vec::new();
        let mut current_section = "";

        for line in stdout.lines() {
            let trimmed = line.trim();

            if trimmed == "Schemes:" {
                current_section = "schemes";
                continue;
            } else if trimmed == "Targets:" {
                current_section = "targets";
                continue;
            } else if trimmed == "Build Configurations:" {
                current_section = "configurations";
                continue;
            } else if trimmed.is_empty() || trimmed.ends_with(':') {
                if !trimmed.is_empty() {
                    current_section = "";
                }
                continue;
            }

            match current_section {
                "schemes" => schemes.push(trimmed.to_string()),
                "targets" => targets.push(trimmed.to_string()),
                "configurations" => configurations.push(trimmed.to_string()),
                _ => {}
            }
        }

        Ok(json!({
            "success": true,
            "path_type": path_type,
            "schemes": schemes,
            "targets": targets,
            "configurations": configurations,
            "scheme_count": schemes.len()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// ShowBuildSettingsTool - Show Xcode build settings
// ============================================================================

pub struct ShowBuildSettingsTool {
    schema: ToolSchema,
}

impl ShowBuildSettingsTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "show_build_settings".to_string(),
                aliases: Some(vec!["xcode_build_settings".to_string()]),
                description: "Show Xcode build settings for a scheme.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj file"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace file"
                        },
                        "scheme": {
                            "type": "string",
                            "description": "Scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "description": "Build configuration (Debug, Release)"
                        },
                        "sdk": {
                            "type": "string",
                            "description": "SDK to use (iphoneos, iphonesimulator, macosx)"
                        },
                        "filter": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Filter to specific settings (e.g., ['PRODUCT_NAME', 'BUNDLE_IDENTIFIER'])"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for ShowBuildSettingsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ShowBuildSettingsTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let mut args = vec!["-showBuildSettings", "-scheme", scheme];

        let workspace;
        let project;
        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            workspace = ws.to_string();
            args.push("-workspace");
            args.push(&workspace);
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            project = proj.to_string();
            args.push("-project");
            args.push(&project);
        }

        let config;
        if let Some(c) = params.get("configuration").and_then(|v| v.as_str()) {
            config = c.to_string();
            args.push("-configuration");
            args.push(&config);
        }

        let sdk;
        if let Some(s) = params.get("sdk").and_then(|v| v.as_str()) {
            sdk = s.to_string();
            args.push("-sdk");
            args.push(&sdk);
        }

        let output = Command::new("xcodebuild")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to run xcodebuild: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(json!({
                "success": false,
                "error": format!("xcodebuild -showBuildSettings failed: {}", stderr)
            }));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse settings into a map
        let filter: Option<Vec<&str>> = params.get("filter").and_then(|v| {
            v.as_array()
                .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect())
        });

        let mut settings = serde_json::Map::new();
        for line in stdout.lines() {
            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim();
                let value = line[eq_pos + 1..].trim();

                if let Some(ref f) = filter {
                    if f.iter().any(|&k| key.contains(k)) {
                        settings.insert(key.to_string(), json!(value));
                    }
                } else {
                    settings.insert(key.to_string(), json!(value));
                }
            }
        }

        // Extract commonly used settings for convenience
        let product_name = settings.get("PRODUCT_NAME").and_then(|v| v.as_str());
        let bundle_id = settings
            .get("PRODUCT_BUNDLE_IDENTIFIER")
            .and_then(|v| v.as_str());
        let built_products_dir = settings.get("BUILT_PRODUCTS_DIR").and_then(|v| v.as_str());

        Ok(json!({
            "success": true,
            "scheme": scheme,
            "product_name": product_name,
            "bundle_identifier": bundle_id,
            "built_products_dir": built_products_dir,
            "settings": settings,
            "settings_count": settings.len()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// GetProjectInfoTool - Get comprehensive project information
// ============================================================================

pub struct GetProjectInfoTool {
    schema: ToolSchema,
}

impl GetProjectInfoTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "get_project_info".to_string(),
                aliases: Some(vec!["project_info".to_string(), "xcode_info".to_string()]),
                description: "Get comprehensive information about an Xcode project or workspace."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": {
                            "type": "string",
                            "description": "Path to .xcodeproj file"
                        },
                        "workspace_path": {
                            "type": "string",
                            "description": "Path to .xcworkspace file"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for GetProjectInfoTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GetProjectInfoTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let mut args = vec!["-list", "-json"];

        let workspace;
        let project;
        let path_type;
        let path_value;

        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            workspace = ws.to_string();
            args.push("-workspace");
            args.push(&workspace);
            path_type = "workspace";
            path_value = ws;
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            project = proj.to_string();
            args.push("-project");
            args.push(&project);
            path_type = "project";
            path_value = proj;
        } else {
            return Ok(json!({
                "success": false,
                "error": "Either 'project_path' or 'workspace_path' is required"
            }));
        }

        let output = Command::new("xcodebuild")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to run xcodebuild: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(json!({
                "success": false,
                "error": format!("xcodebuild -list -json failed: {}", stderr)
            }));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse JSON output
        let json_info: Value = serde_json::from_str(&stdout).unwrap_or(json!({}));

        // Extract info based on project or workspace
        let empty_obj = json!({});
        let (schemes, targets, configurations) = if path_type == "workspace" {
            let workspace_info = json_info.get("workspace").unwrap_or(&empty_obj);
            let schemes: Vec<String> = workspace_info
                .get("schemes")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            (schemes, Vec::new(), Vec::new())
        } else {
            let project_info = json_info.get("project").unwrap_or(&empty_obj);
            let schemes: Vec<String> = project_info
                .get("schemes")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let targets: Vec<String> = project_info
                .get("targets")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let configurations: Vec<String> = project_info
                .get("configurations")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            (schemes, targets, configurations)
        };

        Ok(json!({
            "success": true,
            "path_type": path_type,
            "path": path_value,
            "schemes": schemes,
            "targets": targets,
            "configurations": configurations,
            "scheme_count": schemes.len(),
            "target_count": targets.len(),
            "raw_info": json_info
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_projects_tool_schema() {
        let tool = DiscoverProjectsTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "discover_projects");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"discover_projs".to_string())
        );
    }

    #[test]
    fn test_get_app_bundle_id_tool_schema() {
        let tool = GetAppBundleIdTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "get_app_bundle_id");
    }

    #[test]
    fn test_list_schemes_tool_schema() {
        let tool = ListSchemesTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "list_schemes");
    }

    #[test]
    fn test_show_build_settings_tool_schema() {
        let tool = ShowBuildSettingsTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "show_build_settings");
    }

    #[test]
    fn test_get_project_info_tool_schema() {
        let tool = GetProjectInfoTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "get_project_info");
    }
}
