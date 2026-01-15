use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Execute swift command and return output
async fn run_swift(args: &[&str], working_dir: Option<&str>, timeout_secs: u64) -> Result<(bool, String, String, u64)> {
    let start = Instant::now();

    let mut cmd = Command::new("swift");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let timeout_duration = Duration::from_secs(timeout_secs.min(600));

    let child = cmd
        .spawn()
        .map_err(|e| ToolError::Execution(format!("Failed to run swift: {}", e)))?;

    let output = tokio::time::timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| ToolError::Execution(format!("Command timed out after {} seconds", timeout_secs)))?
        .map_err(|e| ToolError::Execution(format!("Swift command failed: {}", e)))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok((output.status.success(), stdout, stderr, duration_ms))
}

// ============================================================================
// SpmBuildTool - Build a Swift package
// ============================================================================

pub struct SpmBuildTool {
    schema: ToolSchema,
}

impl SpmBuildTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "spm_build".to_string(),
                aliases: Some(vec!["swift_build".to_string()]),
                description: "Build a Swift package using Swift Package Manager.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the directory containing Package.swift"
                        },
                        "target": {
                            "type": "string",
                            "description": "Specific target to build"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["debug", "release"],
                            "description": "Build configuration (default: debug)"
                        },
                        "arch": {
                            "type": "string",
                            "enum": ["arm64", "x86_64"],
                            "description": "Target architecture"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Timeout in seconds (default: 300)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SpmBuildTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpmBuildTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params.get("package_path").and_then(|v| v.as_str());
        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(300);

        let mut args = vec!["build"];

        let target;
        if let Some(t) = params.get("target").and_then(|v| v.as_str()) {
            target = t.to_string();
            args.push("--target");
            args.push(&target);
        }

        let config;
        if let Some(c) = params.get("configuration").and_then(|v| v.as_str()) {
            config = format!("--configuration {}", c);
            args.push("-c");
            args.push(if c == "release" { "release" } else { "debug" });
        }

        let arch;
        if let Some(a) = params.get("arch").and_then(|v| v.as_str()) {
            arch = a.to_string();
            args.push("--arch");
            args.push(&arch);
        }

        let _ = config;
        let (success, stdout, stderr, duration_ms) = run_swift(&args, package_path, timeout_secs).await?;

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
// SpmRunTool - Run a Swift package executable
// ============================================================================

pub struct SpmRunTool {
    schema: ToolSchema,
}

impl SpmRunTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "spm_run".to_string(),
                aliases: Some(vec!["swift_run".to_string()]),
                description: "Run a Swift package executable.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the directory containing Package.swift"
                        },
                        "target": {
                            "type": "string",
                            "description": "Executable target to run"
                        },
                        "arguments": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Arguments to pass to the executable"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["debug", "release"],
                            "description": "Build configuration (default: debug)"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Timeout in seconds (default: 60)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SpmRunTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpmRunTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params.get("package_path").and_then(|v| v.as_str());
        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(60);

        let mut args = vec!["run"];

        let target;
        if let Some(t) = params.get("target").and_then(|v| v.as_str()) {
            target = t.to_string();
            args.push(&target);
        }

        let config;
        if let Some(c) = params.get("configuration").and_then(|v| v.as_str()) {
            config = c.to_string();
            args.push("-c");
            args.push(&config);
        }

        // Add executable arguments after --
        let exec_args: Vec<String>;
        if let Some(arguments) = params.get("arguments").and_then(|v| v.as_array()) {
            exec_args = arguments
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !exec_args.is_empty() {
                args.push("--");
                for arg in &exec_args {
                    args.push(arg);
                }
            }
        }

        let (success, stdout, stderr, duration_ms) = run_swift(&args, package_path, timeout_secs).await?;

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
// SpmTestTool - Run Swift package tests
// ============================================================================

pub struct SpmTestTool {
    schema: ToolSchema,
}

impl SpmTestTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "spm_test".to_string(),
                aliases: Some(vec!["swift_test".to_string()]),
                description: "Run Swift package tests.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the directory containing Package.swift"
                        },
                        "filter": {
                            "type": "string",
                            "description": "Filter to run specific tests (e.g., 'MyTests.testFoo')"
                        },
                        "parallel": {
                            "type": "boolean",
                            "description": "Run tests in parallel (default: true)"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Timeout in seconds (default: 300)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SpmTestTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpmTestTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params.get("package_path").and_then(|v| v.as_str());
        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(300);

        let mut args = vec!["test"];

        let filter;
        if let Some(f) = params.get("filter").and_then(|v| v.as_str()) {
            filter = f.to_string();
            args.push("--filter");
            args.push(&filter);
        }

        let parallel = params.get("parallel").and_then(|v| v.as_bool()).unwrap_or(true);
        if !parallel {
            args.push("--no-parallel");
        }

        let (success, stdout, stderr, duration_ms) = run_swift(&args, package_path, timeout_secs).await?;

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
// SpmCleanTool - Clean Swift package build artifacts
// ============================================================================

pub struct SpmCleanTool {
    schema: ToolSchema,
}

impl SpmCleanTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "spm_clean".to_string(),
                aliases: Some(vec!["swift_clean".to_string()]),
                description: "Clean Swift package build artifacts.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the directory containing Package.swift"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SpmCleanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpmCleanTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params.get("package_path").and_then(|v| v.as_str());

        let args = vec!["package", "clean"];
        let (success, stdout, stderr, duration_ms) = run_swift(&args, package_path, 60).await?;

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
// SpmListTool - List Swift package targets
// ============================================================================

pub struct SpmListTool {
    schema: ToolSchema,
}

impl SpmListTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "spm_list".to_string(),
                aliases: Some(vec!["swift_list".to_string(), "package_targets".to_string()]),
                description: "List targets and products in a Swift package.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the directory containing Package.swift"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for SpmListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpmListTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params.get("package_path").and_then(|v| v.as_str());

        let args = vec!["package", "describe", "--type", "json"];
        let (success, stdout, stderr, duration_ms) = run_swift(&args, package_path, 30).await?;

        if !success {
            return Ok(json!({
                "success": false,
                "error": stderr,
                "duration_ms": duration_ms
            }));
        }

        // Parse JSON output
        let package_info: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| {
            json!({ "raw_output": stdout })
        });

        Ok(json!({
            "success": true,
            "package": package_info,
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
    fn test_spm_build_tool_schema() {
        let tool = SpmBuildTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "spm_build");
        assert!(schema.aliases.as_ref().unwrap().contains(&"swift_build".to_string()));
    }

    #[test]
    fn test_spm_run_tool_schema() {
        let tool = SpmRunTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "spm_run");
    }

    #[test]
    fn test_spm_test_tool_schema() {
        let tool = SpmTestTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "spm_test");
    }

    #[test]
    fn test_spm_clean_tool_schema() {
        let tool = SpmCleanTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "spm_clean");
    }

    #[test]
    fn test_spm_list_tool_schema() {
        let tool = SpmListTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "spm_list");
        assert!(schema.aliases.as_ref().unwrap().contains(&"package_targets".to_string()));
    }
}
