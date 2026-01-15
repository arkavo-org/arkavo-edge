use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Execute xcodebuild command with timeout
async fn run_xcodebuild(
    args: &[&str],
    timeout_secs: u64,
) -> Result<(bool, String, String, u64)> {
    let start = Instant::now();

    let mut cmd = Command::new("xcodebuild");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let timeout_duration = Duration::from_secs(timeout_secs.min(600));

    let child = cmd
        .spawn()
        .map_err(|e| ToolError::Execution(format!("Failed to run xcodebuild: {}", e)))?;

    let output = tokio::time::timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| ToolError::Execution(format!("Build timed out after {} seconds", timeout_secs)))?
        .map_err(|e| ToolError::Execution(format!("xcodebuild failed: {}", e)))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok((output.status.success(), stdout, stderr, duration_ms))
}

// ============================================================================
// XcodeBuildSimTool - Build for iOS simulator
// ============================================================================

pub struct XcodeBuildSimTool {
    schema: ToolSchema,
}

impl XcodeBuildSimTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_build_sim".to_string(),
                aliases: Some(vec!["build_sim".to_string()]),
                description: "Build an Xcode project for iOS simulator.".to_string(),
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
                            "description": "Build scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["Debug", "Release"],
                            "description": "Build configuration (default: Debug)"
                        },
                        "simulator_id": {
                            "type": "string",
                            "description": "Target simulator UUID"
                        },
                        "derived_data_path": {
                            "type": "string",
                            "description": "Custom derived data path"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Build timeout in seconds (default: 300)"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for XcodeBuildSimTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeBuildSimTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params.get("scheme").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'scheme' is required".to_string())
        })?;

        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(300);

        let mut args = vec!["build", "-scheme", scheme, "-sdk", "iphonesimulator"];

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

        let destination;
        if let Some(sim_id) = params.get("simulator_id").and_then(|v| v.as_str()) {
            destination = format!("platform=iOS Simulator,id={}", sim_id);
            args.push("-destination");
            args.push(&destination);
        }

        let derived_data;
        if let Some(dd) = params.get("derived_data_path").and_then(|v| v.as_str()) {
            derived_data = dd.to_string();
            args.push("-derivedDataPath");
            args.push(&derived_data);
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        Ok(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// XcodeBuildDeviceTool - Build for physical device
// ============================================================================

pub struct XcodeBuildDeviceTool {
    schema: ToolSchema,
}

impl XcodeBuildDeviceTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_build_device".to_string(),
                aliases: Some(vec!["build_device".to_string()]),
                description: "Build an Xcode project for a physical iOS device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" },
                        "workspace_path": { "type": "string" },
                        "scheme": { "type": "string" },
                        "configuration": { "type": "string", "enum": ["Debug", "Release"] },
                        "device_id": { "type": "string", "description": "Device UDID" },
                        "derived_data_path": { "type": "string" },
                        "timeout_secs": { "type": "integer" }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for XcodeBuildDeviceTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeBuildDeviceTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params.get("scheme").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'scheme' is required".to_string())
        })?;

        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(300);

        let mut args = vec!["build", "-scheme", scheme, "-sdk", "iphoneos"];

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

        let destination;
        if let Some(device_id) = params.get("device_id").and_then(|v| v.as_str()) {
            destination = format!("platform=iOS,id={}", device_id);
            args.push("-destination");
            args.push(&destination);
        }

        let derived_data;
        if let Some(dd) = params.get("derived_data_path").and_then(|v| v.as_str()) {
            derived_data = dd.to_string();
            args.push("-derivedDataPath");
            args.push(&derived_data);
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        Ok(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// XcodeBuildMacosTool - Build for macOS
// ============================================================================

pub struct XcodeBuildMacosTool {
    schema: ToolSchema,
}

impl XcodeBuildMacosTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_build_macos".to_string(),
                aliases: Some(vec!["build_macos".to_string()]),
                description: "Build an Xcode project for macOS.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" },
                        "workspace_path": { "type": "string" },
                        "scheme": { "type": "string" },
                        "configuration": { "type": "string", "enum": ["Debug", "Release"] },
                        "arch": { "type": "string", "enum": ["arm64", "x86_64"] },
                        "derived_data_path": { "type": "string" },
                        "timeout_secs": { "type": "integer" }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for XcodeBuildMacosTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeBuildMacosTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params.get("scheme").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'scheme' is required".to_string())
        })?;

        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(300);

        let mut args = vec!["build", "-scheme", scheme, "-sdk", "macosx"];

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

        let arch;
        if let Some(a) = params.get("arch").and_then(|v| v.as_str()) {
            arch = a.to_string();
            args.push("-arch");
            args.push(&arch);
        }

        let derived_data;
        if let Some(dd) = params.get("derived_data_path").and_then(|v| v.as_str()) {
            derived_data = dd.to_string();
            args.push("-derivedDataPath");
            args.push(&derived_data);
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        Ok(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// XcodeCleanTool - Clean build artifacts
// ============================================================================

pub struct XcodeCleanTool {
    schema: ToolSchema,
}

impl XcodeCleanTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_clean".to_string(),
                aliases: Some(vec!["clean_build".to_string()]),
                description: "Clean Xcode build artifacts.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" },
                        "workspace_path": { "type": "string" },
                        "scheme": { "type": "string" },
                        "configuration": { "type": "string" }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for XcodeCleanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeCleanTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params.get("scheme").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'scheme' is required".to_string())
        })?;

        let mut args = vec!["clean", "-scheme", scheme];

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

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, 120).await?;

        Ok(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// XcodeTestTool - Run tests
// ============================================================================

pub struct XcodeTestTool {
    schema: ToolSchema,
}

impl XcodeTestTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xcode_test".to_string(),
                aliases: Some(vec!["run_tests".to_string()]),
                description: "Run Xcode tests on simulator or device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "project_path": { "type": "string" },
                        "workspace_path": { "type": "string" },
                        "scheme": { "type": "string" },
                        "configuration": { "type": "string" },
                        "simulator_id": { "type": "string" },
                        "device_id": { "type": "string" },
                        "test_plan": { "type": "string", "description": "Test plan name" },
                        "only_testing": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Run only specific tests"
                        },
                        "timeout_secs": { "type": "integer" }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for XcodeTestTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for XcodeTestTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params.get("scheme").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'scheme' is required".to_string())
        })?;

        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(600);

        let mut args = vec!["test", "-scheme", scheme];

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

        let destination;
        if let Some(sim_id) = params.get("simulator_id").and_then(|v| v.as_str()) {
            destination = format!("platform=iOS Simulator,id={}", sim_id);
            args.push("-destination");
            args.push(&destination);
        } else if let Some(device_id) = params.get("device_id").and_then(|v| v.as_str()) {
            destination = format!("platform=iOS,id={}", device_id);
            args.push("-destination");
            args.push(&destination);
        }

        let test_plan;
        if let Some(tp) = params.get("test_plan").and_then(|v| v.as_str()) {
            test_plan = tp.to_string();
            args.push("-testPlan");
            args.push(&test_plan);
        }

        let only_testing: Vec<String>;
        if let Some(tests) = params.get("only_testing").and_then(|v| v.as_array()) {
            only_testing = tests
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for test in &only_testing {
                args.push("-only-testing");
                args.push(test);
            }
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        Ok(json!({
            "success": success,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms
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
    fn test_xcode_build_sim_tool_schema() {
        let tool = XcodeBuildSimTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_build_sim");
        assert!(schema.aliases.as_ref().unwrap().contains(&"build_sim".to_string()));
    }

    #[test]
    fn test_xcode_build_device_tool_schema() {
        let tool = XcodeBuildDeviceTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_build_device");
    }

    #[test]
    fn test_xcode_build_macos_tool_schema() {
        let tool = XcodeBuildMacosTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_build_macos");
    }

    #[test]
    fn test_xcode_clean_tool_schema() {
        let tool = XcodeCleanTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_clean");
    }

    #[test]
    fn test_xcode_test_tool_schema() {
        let tool = XcodeTestTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "xcode_test");
        assert!(schema.aliases.as_ref().unwrap().contains(&"run_tests".to_string()));
    }
}
