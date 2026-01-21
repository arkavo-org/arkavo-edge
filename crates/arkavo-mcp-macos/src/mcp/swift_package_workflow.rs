use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

// Global store for running Swift package processes
lazy_static::lazy_static! {
    static ref SPM_PROCESSES: Arc<Mutex<HashMap<u32, SpmProcessInfo>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

/// Information about a running Swift package process
struct SpmProcessInfo {
    executable_name: String,
    package_path: String,
    start_time: Instant,
    child: Option<Child>,
}

/// Run swift command with optional timeout
async fn run_swift(args: &[&str], timeout_secs: u64) -> Result<(bool, String, String, u64)> {
    let start = Instant::now();

    let mut cmd = Command::new("swift");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let timeout_duration = Duration::from_secs(timeout_secs.min(600));

    let child = cmd
        .spawn()
        .map_err(|e| TestError::Execution(format!("Failed to run swift: {}", e)))?;

    let output = tokio::time::timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| {
            TestError::Execution(format!("Swift timed out after {} seconds", timeout_secs))
        })?
        .map_err(|e| TestError::Execution(format!("swift failed: {}", e)))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok((output.status.success(), stdout, stderr, duration_ms))
}

// ============================================================================
// SwiftPackageBuildTool - Build Swift package
// ============================================================================

pub struct SwiftPackageBuildTool {
    schema: ToolSchema,
}

impl SwiftPackageBuildTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swift_package_build".to_string(),
                aliases: Some(vec!["spm_build".to_string()]),
                description: "Build a Swift package using swift build.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the Swift package root"
                        },
                        "target_name": {
                            "type": "string",
                            "description": "Specific target to build"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["debug", "release"],
                            "description": "Build configuration (default: debug)"
                        },
                        "architectures": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Target architectures (e.g., arm64, x86_64)"
                        },
                        "parse_as_library": {
                            "type": "boolean",
                            "description": "Build as library instead of executable"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Build timeout in seconds (default: 300)"
                        }
                    },
                    "required": ["package_path"]
                }),
            },
        }
    }
}

impl Default for SwiftPackageBuildTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SwiftPackageBuildTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params
            .get("package_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'package_path' is required".to_string()))?;

        // Verify path exists
        if !Path::new(package_path).exists() {
            return Ok(json!({
                "success": false,
                "error": format!("Package path does not exist: {}", package_path)
            }));
        }

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        let mut args = vec!["build", "--package-path", package_path];

        let config;
        if params
            .get("configuration")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c == "release")
        {
            config = "release".to_string();
            args.push("-c");
            args.push(&config);
        }

        let target;
        if let Some(t) = params.get("target_name").and_then(|v| v.as_str()) {
            target = t.to_string();
            args.push("--target");
            args.push(&target);
        }

        let arch_strings: Vec<String>;
        if let Some(archs) = params.get("architectures").and_then(|v| v.as_array()) {
            arch_strings = archs
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for arch in &arch_strings {
                args.push("--arch");
                args.push(arch);
            }
        }

        if params
            .get("parse_as_library")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            args.push("-Xswiftc");
            args.push("-parse-as-library");
        }

        let (success, stdout, stderr, duration_ms) = run_swift(&args, timeout_secs).await?;

        Ok(json!({
            "success": success,
            "package_path": package_path,
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
// SwiftPackageRunTool - Run Swift package executable
// ============================================================================

pub struct SwiftPackageRunTool {
    schema: ToolSchema,
}

impl SwiftPackageRunTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swift_package_run".to_string(),
                aliases: Some(vec!["spm_run".to_string()]),
                description: "Run an executable from a Swift package.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the Swift package root"
                        },
                        "executable_name": {
                            "type": "string",
                            "description": "Name of executable to run (defaults to package name)"
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
                        "background": {
                            "type": "boolean",
                            "description": "Run in background and return immediately (default: false)"
                        },
                        "parse_as_library": {
                            "type": "boolean",
                            "description": "Add -parse-as-library flag for @main support"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Timeout in seconds for foreground execution (default: 30)"
                        }
                    },
                    "required": ["package_path"]
                }),
            },
        }
    }
}

impl Default for SwiftPackageRunTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SwiftPackageRunTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params
            .get("package_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'package_path' is required".to_string()))?;

        if !Path::new(package_path).exists() {
            return Ok(json!({
                "success": false,
                "error": format!("Package path does not exist: {}", package_path)
            }));
        }

        let background = params
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        let executable_name = params
            .get("executable_name")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let mut args = vec![
            "run".to_string(),
            "--package-path".to_string(),
            package_path.to_string(),
        ];

        if params
            .get("configuration")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c == "release")
        {
            args.push("-c".to_string());
            args.push("release".to_string());
        }

        if params
            .get("parse_as_library")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            args.push("-Xswiftc".to_string());
            args.push("-parse-as-library".to_string());
        }

        if let Some(exec) = params.get("executable_name").and_then(|v| v.as_str()) {
            args.push(exec.to_string());
        }

        if let Some(exec_args) = params.get("arguments").and_then(|v| v.as_array()) {
            args.push("--".to_string());
            for arg in exec_args {
                if let Some(s) = arg.as_str() {
                    args.push(s.to_string());
                }
            }
        }

        if background {
            // Start process in background
            let mut cmd = Command::new("swift");
            cmd.args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let child = cmd
                .spawn()
                .map_err(|e| TestError::Execution(format!("Failed to start swift run: {}", e)))?;

            let pid = child.id().unwrap_or(0);

            // Store process info
            {
                let mut processes = SPM_PROCESSES.lock().await;
                processes.insert(
                    pid,
                    SpmProcessInfo {
                        executable_name: executable_name.to_string(),
                        package_path: package_path.to_string(),
                        start_time: Instant::now(),
                        child: Some(child),
                    },
                );
            }

            Ok(json!({
                "success": true,
                "background": true,
                "pid": pid,
                "package_path": package_path,
                "executable_name": executable_name,
                "message": format!("Started executable in background (PID: {})", pid)
            }))
        } else {
            // Run in foreground with timeout
            let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let (success, stdout, stderr, duration_ms) =
                run_swift(&args_refs, timeout_secs).await?;

            Ok(json!({
                "success": success,
                "background": false,
                "package_path": package_path,
                "stdout": stdout,
                "stderr": stderr,
                "duration_ms": duration_ms
            }))
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SwiftPackageTestTool - Run Swift package tests
// ============================================================================

pub struct SwiftPackageTestTool {
    schema: ToolSchema,
}

impl SwiftPackageTestTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swift_package_test".to_string(),
                aliases: Some(vec!["spm_test".to_string()]),
                description: "Run tests for a Swift package.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the Swift package root"
                        },
                        "test_product": {
                            "type": "string",
                            "description": "Specific test product to run"
                        },
                        "filter": {
                            "type": "string",
                            "description": "Filter tests by name (regex pattern)"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["debug", "release"],
                            "description": "Build configuration (default: debug)"
                        },
                        "parallel": {
                            "type": "boolean",
                            "description": "Run tests in parallel (default: true)"
                        },
                        "show_codecov": {
                            "type": "boolean",
                            "description": "Show code coverage (default: false)"
                        },
                        "parse_as_library": {
                            "type": "boolean",
                            "description": "Add -parse-as-library flag"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Test timeout in seconds (default: 600)"
                        }
                    },
                    "required": ["package_path"]
                }),
            },
        }
    }
}

impl Default for SwiftPackageTestTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SwiftPackageTestTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params
            .get("package_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'package_path' is required".to_string()))?;

        if !Path::new(package_path).exists() {
            return Ok(json!({
                "success": false,
                "error": format!("Package path does not exist: {}", package_path)
            }));
        }

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(600);

        let mut args = vec!["test", "--package-path", package_path];

        let config;
        if params
            .get("configuration")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c == "release")
        {
            config = "release".to_string();
            args.push("-c");
            args.push(&config);
        }

        let test_product;
        if let Some(tp) = params.get("test_product").and_then(|v| v.as_str()) {
            test_product = tp.to_string();
            args.push("--test-product");
            args.push(&test_product);
        }

        let filter;
        if let Some(f) = params.get("filter").and_then(|v| v.as_str()) {
            filter = f.to_string();
            args.push("--filter");
            args.push(&filter);
        }

        if params
            .get("parallel")
            .and_then(|v| v.as_bool())
            .map(|p| !p)
            .unwrap_or(false)
        {
            args.push("--no-parallel");
        }

        if params
            .get("show_codecov")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            args.push("--show-code-coverage");
        }

        if params
            .get("parse_as_library")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            args.push("-Xswiftc");
            args.push("-parse-as-library");
        }

        let (success, stdout, stderr, duration_ms) = run_swift(&args, timeout_secs).await?;

        // Parse test results
        let tests_passed = stdout.contains("Test Suite") && stdout.contains("passed");
        let test_count = stdout.lines().filter(|l| l.contains("Test Case")).count();

        Ok(json!({
            "success": success && tests_passed,
            "package_path": package_path,
            "tests_run": test_count,
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
// SwiftPackageCleanTool - Clean Swift package
// ============================================================================

pub struct SwiftPackageCleanTool {
    schema: ToolSchema,
}

impl SwiftPackageCleanTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swift_package_clean".to_string(),
                aliases: Some(vec!["spm_clean".to_string()]),
                description: "Clean Swift package build artifacts and derived data.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "package_path": {
                            "type": "string",
                            "description": "Path to the Swift package root"
                        }
                    },
                    "required": ["package_path"]
                }),
            },
        }
    }
}

impl Default for SwiftPackageCleanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SwiftPackageCleanTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let package_path = params
            .get("package_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'package_path' is required".to_string()))?;

        if !Path::new(package_path).exists() {
            return Ok(json!({
                "success": false,
                "error": format!("Package path does not exist: {}", package_path)
            }));
        }

        let args = ["package", "--package-path", package_path, "clean"];

        let (success, stdout, stderr, duration_ms) = run_swift(&args, 60).await?;

        Ok(json!({
            "success": success,
            "package_path": package_path,
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms,
            "message": if success { "Build artifacts and derived data removed" } else { "Clean failed" }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SwiftPackageStopTool - Stop running Swift package process
// ============================================================================

pub struct SwiftPackageStopTool {
    schema: ToolSchema,
}

impl SwiftPackageStopTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swift_package_stop".to_string(),
                aliases: Some(vec!["spm_stop".to_string()]),
                description: "Stop a running Swift package executable.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pid": {
                            "type": "integer",
                            "description": "Process ID (PID) of the running executable"
                        }
                    },
                    "required": ["pid"]
                }),
            },
        }
    }
}

impl Default for SwiftPackageStopTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SwiftPackageStopTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let pid = params
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| TestError::Mcp("'pid' is required".to_string()))?
            as u32;

        let mut processes = SPM_PROCESSES.lock().await;

        if let Some(mut info) = processes.remove(&pid) {
            let runtime_secs = info.start_time.elapsed().as_secs();

            // Kill the child process if we have it
            if let Some(mut child) = info.child.take() {
                let _ = child.kill().await;
            } else {
                // Try to kill by PID directly
                let _ = Command::new("kill").arg(pid.to_string()).output().await;
            }

            Ok(json!({
                "success": true,
                "pid": pid,
                "runtime_secs": runtime_secs,
                "executable_name": info.executable_name,
                "package_path": info.package_path,
                "message": format!("Stopped executable (was running for {}s)", runtime_secs)
            }))
        } else {
            // Process not in our tracking - try to kill anyway
            let output = Command::new("kill")
                .arg(pid.to_string())
                .output()
                .await
                .map_err(|e| TestError::Execution(format!("Failed to kill process: {}", e)))?;

            Ok(json!({
                "success": output.status.success(),
                "pid": pid,
                "message": if output.status.success() {
                    "Process terminated (was not tracked)"
                } else {
                    "Process not found or already terminated"
                }
            }))
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SwiftPackageListTool - List running Swift package processes
// ============================================================================

pub struct SwiftPackageListTool {
    schema: ToolSchema,
}

impl SwiftPackageListTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swift_package_list".to_string(),
                aliases: Some(vec!["spm_list".to_string()]),
                description: "List currently running Swift package processes.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        }
    }
}

impl Default for SwiftPackageListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SwiftPackageListTool {
    async fn execute(&self, _params: Value) -> Result<Value> {
        let processes = SPM_PROCESSES.lock().await;

        if processes.is_empty() {
            return Ok(json!({
                "success": true,
                "count": 0,
                "processes": [],
                "message": "No Swift package processes currently running"
            }));
        }

        let process_list: Vec<Value> = processes
            .iter()
            .map(|(pid, info)| {
                json!({
                    "pid": pid,
                    "executable_name": info.executable_name,
                    "package_path": info.package_path,
                    "runtime_secs": info.start_time.elapsed().as_secs()
                })
            })
            .collect();

        Ok(json!({
            "success": true,
            "count": process_list.len(),
            "processes": process_list
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// SwiftPackageInitTool - Initialize a new Swift package
// ============================================================================

pub struct SwiftPackageInitTool {
    schema: ToolSchema,
}

impl SwiftPackageInitTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swift_package_init".to_string(),
                aliases: Some(vec!["spm_init".to_string()]),
                description: "Initialize a new Swift package.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory where to create the package"
                        },
                        "name": {
                            "type": "string",
                            "description": "Package name (defaults to directory name)"
                        },
                        "type": {
                            "type": "string",
                            "enum": ["library", "executable", "tool", "build-tool-plugin", "command-plugin", "macro"],
                            "description": "Package type (default: library)"
                        }
                    },
                    "required": ["path"]
                }),
            },
        }
    }
}

impl Default for SwiftPackageInitTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SwiftPackageInitTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'path' is required".to_string()))?;

        // Create directory if it doesn't exist
        if !Path::new(path).exists() {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|e| TestError::Execution(format!("Failed to create directory: {}", e)))?;
        }

        let mut args = vec!["package", "init"];

        let package_type;
        if let Some(t) = params.get("type").and_then(|v| v.as_str()) {
            package_type = t.to_string();
            args.push("--type");
            args.push(&package_type);
        }

        let name;
        if let Some(n) = params.get("name").and_then(|v| v.as_str()) {
            name = n.to_string();
            args.push("--name");
            args.push(&name);
        }

        // Run swift package init in the target directory
        let output = Command::new("swift")
            .args(&args)
            .current_dir(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                TestError::Execution(format!("Failed to run swift package init: {}", e))
            })?;

        let success = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(json!({
            "success": success,
            "path": path,
            "stdout": stdout,
            "stderr": stderr
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
    fn test_swift_package_build_tool_schema() {
        let tool = SwiftPackageBuildTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "swift_package_build");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"spm_build".to_string())
        );
    }

    #[test]
    fn test_swift_package_run_tool_schema() {
        let tool = SwiftPackageRunTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "swift_package_run");
    }

    #[test]
    fn test_swift_package_test_tool_schema() {
        let tool = SwiftPackageTestTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "swift_package_test");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"spm_test".to_string())
        );
    }

    #[test]
    fn test_swift_package_clean_tool_schema() {
        let tool = SwiftPackageCleanTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "swift_package_clean");
    }

    #[test]
    fn test_swift_package_stop_tool_schema() {
        let tool = SwiftPackageStopTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "swift_package_stop");
    }

    #[test]
    fn test_swift_package_list_tool_schema() {
        let tool = SwiftPackageListTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "swift_package_list");
    }

    #[test]
    fn test_swift_package_init_tool_schema() {
        let tool = SwiftPackageInitTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "swift_package_init");
    }
}
