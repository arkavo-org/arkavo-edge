use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Execute xcodebuild command with timeout
async fn run_xcodebuild(args: &[&str], timeout_secs: u64) -> Result<(bool, String, String, u64)> {
    let start = Instant::now();

    let mut cmd = Command::new("xcodebuild");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let timeout_duration = Duration::from_secs(timeout_secs.min(600));

    let child = cmd
        .spawn()
        .map_err(|e| TestError::Execution(format!("Failed to run xcodebuild: {}", e)))?;

    let output = tokio::time::timeout(timeout_duration, child.wait_with_output())
        .await
        .map_err(|_| {
            TestError::Execution(format!("Build timed out after {} seconds", timeout_secs))
        })?
        .map_err(|e| TestError::Execution(format!("xcodebuild failed: {}", e)))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok((output.status.success(), stdout, stderr, duration_ms))
}

/// Run shell command
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
// MacosBuildTool - Build macOS app
// ============================================================================

pub struct MacosBuildTool {
    schema: ToolSchema,
}

impl MacosBuildTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_build".to_string(),
                aliases: Some(vec!["build_macos".to_string()]),
                description: "Build a macOS application using xcodebuild.".to_string(),
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
                            "description": "Build scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["Debug", "Release"],
                            "description": "Build configuration (default: Debug)"
                        },
                        "arch": {
                            "type": "string",
                            "enum": ["arm64", "x86_64"],
                            "description": "Architecture to build for"
                        },
                        "derived_data_path": {
                            "type": "string",
                            "description": "Custom derived data path"
                        },
                        "extra_args": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Additional xcodebuild arguments"
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

impl Default for MacosBuildTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosBuildTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

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

        let destination;
        if let Some(arch) = params.get("arch").and_then(|v| v.as_str()) {
            destination = format!("platform=macOS,arch={}", arch);
            args.push("-destination");
            args.push(&destination);
        }

        let derived_data;
        if let Some(dd) = params.get("derived_data_path").and_then(|v| v.as_str()) {
            derived_data = dd.to_string();
            args.push("-derivedDataPath");
            args.push(&derived_data);
        }

        let extra_args_vec: Vec<String>;
        if let Some(extra) = params.get("extra_args").and_then(|v| v.as_array()) {
            extra_args_vec = extra
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for arg in &extra_args_vec {
                args.push(arg);
            }
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        Ok(json!({
            "success": success,
            "scheme": scheme,
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
// MacosBuildRunTool - Build and run macOS app
// ============================================================================

pub struct MacosBuildRunTool {
    schema: ToolSchema,
}

impl MacosBuildRunTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_build_run".to_string(),
                aliases: Some(vec!["build_run_macos".to_string()]),
                description: "Build and run a macOS application in one step.".to_string(),
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
                            "description": "Build scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["Debug", "Release"],
                            "description": "Build configuration (default: Debug)"
                        },
                        "arch": {
                            "type": "string",
                            "enum": ["arm64", "x86_64"],
                            "description": "Architecture to build for"
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

impl Default for MacosBuildRunTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosBuildRunTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        // First, build the app
        let mut build_args = vec!["build", "-scheme", scheme, "-sdk", "macosx"];

        let workspace;
        let project;
        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            workspace = ws.to_string();
            build_args.push("-workspace");
            build_args.push(&workspace);
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            project = proj.to_string();
            build_args.push("-project");
            build_args.push(&project);
        }

        let config;
        let config_value = params
            .get("configuration")
            .and_then(|v| v.as_str())
            .unwrap_or("Debug");
        config = config_value.to_string();
        build_args.push("-configuration");
        build_args.push(&config);

        let destination;
        if let Some(arch) = params.get("arch").and_then(|v| v.as_str()) {
            destination = format!("platform=macOS,arch={}", arch);
            build_args.push("-destination");
            build_args.push(&destination);
        }

        let derived_data;
        if let Some(dd) = params.get("derived_data_path").and_then(|v| v.as_str()) {
            derived_data = dd.to_string();
            build_args.push("-derivedDataPath");
            build_args.push(&derived_data);
        }

        let (build_success, build_stdout, build_stderr, build_duration) =
            run_xcodebuild(&build_args, timeout_secs).await?;

        if !build_success {
            return Ok(json!({
                "success": false,
                "phase": "build",
                "scheme": scheme,
                "stdout": build_stdout,
                "stderr": build_stderr,
                "duration_ms": build_duration
            }));
        }

        // Get app path from build settings
        let mut settings_args = vec!["-showBuildSettings", "-scheme", scheme, "-sdk", "macosx"];

        let ws_clone;
        let proj_clone;
        if let Some(ws) = params.get("workspace_path").and_then(|v| v.as_str()) {
            ws_clone = ws.to_string();
            settings_args.push("-workspace");
            settings_args.push(&ws_clone);
        } else if let Some(proj) = params.get("project_path").and_then(|v| v.as_str()) {
            proj_clone = proj.to_string();
            settings_args.push("-project");
            settings_args.push(&proj_clone);
        }

        settings_args.push("-configuration");
        settings_args.push(&config);

        let (_, settings_output, _, _) = run_xcodebuild(&settings_args, 60).await?;

        // Parse BUILT_PRODUCTS_DIR and FULL_PRODUCT_NAME
        let built_products_dir = settings_output
            .lines()
            .find(|l| l.contains("BUILT_PRODUCTS_DIR"))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().to_string());

        let full_product_name = settings_output
            .lines()
            .find(|l| l.contains("FULL_PRODUCT_NAME"))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().to_string());

        let app_path = match (built_products_dir, full_product_name) {
            (Some(dir), Some(name)) => format!("{}/{}", dir, name),
            _ => {
                return Ok(json!({
                    "success": false,
                    "phase": "app_path",
                    "error": "Could not determine app path from build settings",
                    "build_stdout": build_stdout
                }));
            }
        };

        // Launch the app
        let launch_result = Command::new("open")
            .arg(&app_path)
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to launch app: {}", e)))?;

        let launch_success = launch_result.status.success();

        Ok(json!({
            "success": launch_success,
            "scheme": scheme,
            "app_path": app_path,
            "build_duration_ms": build_duration,
            "build_stdout": build_stdout,
            "build_stderr": build_stderr
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// MacosTestTool - Run macOS tests
// ============================================================================

pub struct MacosTestTool {
    schema: ToolSchema,
}

impl MacosTestTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_test".to_string(),
                aliases: Some(vec!["test_macos".to_string()]),
                description: "Run Xcode tests for a macOS application.".to_string(),
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
                            "description": "Test scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["Debug", "Release"],
                            "description": "Build configuration (default: Debug)"
                        },
                        "test_plan": {
                            "type": "string",
                            "description": "Test plan name"
                        },
                        "only_testing": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Run only specific tests"
                        },
                        "skip_testing": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Skip specific tests"
                        },
                        "result_bundle_path": {
                            "type": "string",
                            "description": "Path to save test results (.xcresult)"
                        },
                        "derived_data_path": {
                            "type": "string",
                            "description": "Custom derived data path"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Test timeout in seconds (default: 600)"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for MacosTestTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosTestTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(600);

        let mut args = vec!["test", "-scheme", scheme, "-sdk", "macosx"];

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

        let test_plan;
        if let Some(tp) = params.get("test_plan").and_then(|v| v.as_str()) {
            test_plan = tp.to_string();
            args.push("-testPlan");
            args.push(&test_plan);
        }

        let result_bundle;
        if let Some(rb) = params.get("result_bundle_path").and_then(|v| v.as_str()) {
            result_bundle = rb.to_string();
            args.push("-resultBundlePath");
            args.push(&result_bundle);
        }

        let derived_data;
        if let Some(dd) = params.get("derived_data_path").and_then(|v| v.as_str()) {
            derived_data = dd.to_string();
            args.push("-derivedDataPath");
            args.push(&derived_data);
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

        let skip_testing: Vec<String>;
        if let Some(tests) = params.get("skip_testing").and_then(|v| v.as_array()) {
            skip_testing = tests
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for test in &skip_testing {
                args.push("-skip-testing");
                args.push(test);
            }
        }

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, timeout_secs).await?;

        // Parse test results
        let tests_passed = stdout.contains("Test Suite") && stdout.contains("passed");
        let test_count = stdout.lines().filter(|l| l.contains("Test Case")).count();
        let failed_count = stdout.lines().filter(|l| l.contains("failed")).count();

        Ok(json!({
            "success": success && tests_passed,
            "scheme": scheme,
            "tests_run": test_count,
            "tests_failed": failed_count,
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
// MacosLaunchTool - Launch macOS app
// ============================================================================

pub struct MacosLaunchTool {
    schema: ToolSchema,
}

impl MacosLaunchTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_launch".to_string(),
                aliases: Some(vec!["launch_mac_app".to_string()]),
                description: "Launch a macOS application using the open command.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "app_path": {
                            "type": "string",
                            "description": "Path to the .app bundle"
                        },
                        "args": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Arguments to pass to the app"
                        },
                        "new_instance": {
                            "type": "boolean",
                            "description": "Launch a new instance even if already running (default: false)"
                        },
                        "background": {
                            "type": "boolean",
                            "description": "Launch in background (default: false)"
                        }
                    },
                    "required": ["app_path"]
                }),
            },
        }
    }
}

impl Default for MacosLaunchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosLaunchTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let app_path = params
            .get("app_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'app_path' is required".to_string()))?;

        // Verify app exists
        if !Path::new(app_path).exists() {
            return Ok(json!({
                "success": false,
                "error": format!("App not found at path: {}", app_path)
            }));
        }

        let mut cmd = Command::new("open");
        cmd.arg(app_path);

        if params
            .get("new_instance")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            cmd.arg("-n");
        }

        if params
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            cmd.arg("-g");
        }

        if let Some(args) = params.get("args").and_then(|v| v.as_array()) {
            cmd.arg("--args");
            for arg in args {
                if let Some(s) = arg.as_str() {
                    cmd.arg(s);
                }
            }
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to launch app: {}", e)))?;

        Ok(json!({
            "success": output.status.success(),
            "app_path": app_path,
            "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
            "stderr": String::from_utf8_lossy(&output.stderr).to_string()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// MacosStopTool - Stop macOS app
// ============================================================================

pub struct MacosStopTool {
    schema: ToolSchema,
}

impl MacosStopTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_stop".to_string(),
                aliases: Some(vec!["stop_mac_app".to_string()]),
                description: "Stop a running macOS application.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "app_name": {
                            "type": "string",
                            "description": "Name of the application to stop"
                        },
                        "process_id": {
                            "type": "integer",
                            "description": "Process ID (PID) to stop"
                        },
                        "graceful": {
                            "type": "boolean",
                            "description": "Use AppleScript quit instead of kill (default: true)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for MacosStopTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosStopTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let app_name = params.get("app_name").and_then(|v| v.as_str());
        let process_id = params.get("process_id").and_then(|v| v.as_i64());
        let graceful = params
            .get("graceful")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if app_name.is_none() && process_id.is_none() {
            return Ok(json!({
                "success": false,
                "error": "Either 'app_name' or 'process_id' is required"
            }));
        }

        if let Some(pid) = process_id {
            // Kill by PID
            let output = Command::new("kill")
                .arg(pid.to_string())
                .output()
                .await
                .map_err(|e| TestError::Execution(format!("Failed to kill process: {}", e)))?;

            return Ok(json!({
                "success": output.status.success(),
                "process_id": pid,
                "method": "kill"
            }));
        }

        if let Some(name) = app_name {
            if graceful {
                // Try AppleScript first
                let script = format!("tell application \"{}\" to quit", name);
                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(&script)
                    .output()
                    .await
                    .map_err(|e| TestError::Execution(format!("Failed to run osascript: {}", e)))?;

                if output.status.success() {
                    return Ok(json!({
                        "success": true,
                        "app_name": name,
                        "method": "applescript"
                    }));
                }
            }

            // Fall back to pkill
            let output = Command::new("pkill")
                .arg("-f")
                .arg(name)
                .output()
                .await
                .map_err(|e| TestError::Execution(format!("Failed to pkill: {}", e)))?;

            return Ok(json!({
                "success": output.status.success(),
                "app_name": name,
                "method": "pkill"
            }));
        }

        Ok(json!({
            "success": false,
            "error": "No valid target specified"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// MacosAppPathTool - Get macOS app path
// ============================================================================

pub struct MacosAppPathTool {
    schema: ToolSchema,
}

impl MacosAppPathTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_app_path".to_string(),
                aliases: Some(vec!["get_mac_app_path".to_string()]),
                description: "Get the built app bundle path for a macOS application.".to_string(),
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
                            "description": "Build scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["Debug", "Release"],
                            "description": "Build configuration (default: Debug)"
                        },
                        "arch": {
                            "type": "string",
                            "enum": ["arm64", "x86_64"],
                            "description": "Architecture"
                        },
                        "derived_data_path": {
                            "type": "string",
                            "description": "Custom derived data path"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for MacosAppPathTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosAppPathTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let mut args = vec!["-showBuildSettings", "-scheme", scheme, "-sdk", "macosx"];

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
        let config_value = params
            .get("configuration")
            .and_then(|v| v.as_str())
            .unwrap_or("Debug");
        config = config_value.to_string();
        args.push("-configuration");
        args.push(&config);

        let destination;
        if let Some(arch) = params.get("arch").and_then(|v| v.as_str()) {
            destination = format!("platform=macOS,arch={}", arch);
            args.push("-destination");
            args.push(&destination);
        }

        let (_, stdout, _, _) = run_xcodebuild(&args, 60).await?;

        // Parse BUILT_PRODUCTS_DIR and FULL_PRODUCT_NAME
        let built_products_dir = stdout
            .lines()
            .find(|l| l.contains("BUILT_PRODUCTS_DIR"))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().to_string());

        let full_product_name = stdout
            .lines()
            .find(|l| l.contains("FULL_PRODUCT_NAME"))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().to_string());

        match (built_products_dir, full_product_name) {
            (Some(dir), Some(name)) => {
                let app_path = format!("{}/{}", dir, name);
                Ok(json!({
                    "success": true,
                    "app_path": app_path,
                    "built_products_dir": dir,
                    "product_name": name
                }))
            }
            _ => Ok(json!({
                "success": false,
                "error": "Could not determine app path from build settings"
            })),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// MacosCleanTool - Clean macOS build
// ============================================================================

pub struct MacosCleanTool {
    schema: ToolSchema,
}

impl MacosCleanTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_clean".to_string(),
                aliases: Some(vec!["clean_macos".to_string()]),
                description: "Clean Xcode build artifacts for macOS.".to_string(),
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
                            "description": "Build scheme name"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for MacosCleanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosCleanTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let mut args = vec!["clean", "-scheme", scheme, "-sdk", "macosx"];

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

        let (success, stdout, stderr, duration_ms) = run_xcodebuild(&args, 120).await?;

        Ok(json!({
            "success": success,
            "scheme": scheme,
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
// MacosBundleIdTool - Get bundle ID
// ============================================================================

pub struct MacosBundleIdTool {
    schema: ToolSchema,
}

impl MacosBundleIdTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_bundle_id".to_string(),
                aliases: Some(vec!["get_mac_bundle_id".to_string()]),
                description: "Get the bundle identifier of a macOS application.".to_string(),
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

impl Default for MacosBundleIdTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosBundleIdTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let app_path = params
            .get("app_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'app_path' is required".to_string()))?;

        let info_plist = format!("{}/Contents/Info.plist", app_path);

        if !Path::new(&info_plist).exists() {
            return Ok(json!({
                "success": false,
                "error": format!("Info.plist not found at: {}", info_plist)
            }));
        }

        let output = run_command(
            "/usr/libexec/PlistBuddy",
            &["-c", "Print :CFBundleIdentifier", &info_plist],
        )
        .await?;

        Ok(json!({
            "success": true,
            "app_path": app_path,
            "bundle_id": output.trim()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// MacosBuildSettingsTool - Show build settings
// ============================================================================

pub struct MacosBuildSettingsTool {
    schema: ToolSchema,
}

impl MacosBuildSettingsTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_build_settings".to_string(),
                aliases: Some(vec!["show_build_settings".to_string()]),
                description: "Show Xcode build settings for a macOS project.".to_string(),
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
                            "description": "Build scheme name"
                        },
                        "configuration": {
                            "type": "string",
                            "enum": ["Debug", "Release"],
                            "description": "Build configuration"
                        },
                        "filter": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Settings to filter (e.g., ['PRODUCT_NAME', 'BUNDLE_ID'])"
                        }
                    },
                    "required": ["scheme"]
                }),
            },
        }
    }
}

impl Default for MacosBuildSettingsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosBuildSettingsTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scheme = params
            .get("scheme")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'scheme' is required".to_string()))?;

        let mut args = vec!["-showBuildSettings", "-scheme", scheme, "-sdk", "macosx"];

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

        let (success, stdout, stderr, _) = run_xcodebuild(&args, 60).await?;

        if !success {
            return Ok(json!({
                "success": false,
                "error": stderr
            }));
        }

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

        Ok(json!({
            "success": true,
            "scheme": scheme,
            "settings": settings
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// MacosListSchemesTool - List available schemes
// ============================================================================

pub struct MacosListSchemesTool {
    schema: ToolSchema,
}

impl MacosListSchemesTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_list_schemes".to_string(),
                aliases: Some(vec!["list_schemes".to_string()]),
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

impl Default for MacosListSchemesTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosListSchemesTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let mut args = vec!["-list"];

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

        let (success, stdout, stderr, _) = run_xcodebuild(&args, 30).await?;

        if !success {
            return Ok(json!({
                "success": false,
                "error": stderr
            }));
        }

        // Parse schemes from output
        let mut schemes = Vec::new();
        let mut in_schemes_section = false;

        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed == "Schemes:" {
                in_schemes_section = true;
                continue;
            }
            if in_schemes_section {
                if trimmed.is_empty() || trimmed.ends_with(':') {
                    break;
                }
                schemes.push(trimmed.to_string());
            }
        }

        Ok(json!({
            "success": true,
            "schemes": schemes,
            "count": schemes.len()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// MacosDiscoverProjectsTool - Discover Xcode projects
// ============================================================================

pub struct MacosDiscoverProjectsTool {
    schema: ToolSchema,
}

impl MacosDiscoverProjectsTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "macos_discover_projects".to_string(),
                aliases: Some(vec!["discover_projs".to_string()]),
                description: "Discover Xcode projects and workspaces in a directory.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory to search (default: current directory)"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Search recursively (default: true)"
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum recursion depth (default: 5)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for MacosDiscoverProjectsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MacosDiscoverProjectsTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let recursive = params
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let max_depth = params
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);

        let mut projects = Vec::new();
        let mut workspaces = Vec::new();

        // Use find command
        let max_depth_str = max_depth.to_string();
        let depth_args: Vec<&str> = if recursive {
            vec!["-maxdepth", &max_depth_str]
        } else {
            vec!["-maxdepth", "1"]
        };

        // Find .xcodeproj
        let find_output = Command::new("find")
            .arg(path)
            .args(&depth_args)
            .arg("-name")
            .arg("*.xcodeproj")
            .arg("-type")
            .arg("d")
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to find projects: {}", e)))?;

        if find_output.status.success() {
            let output = String::from_utf8_lossy(&find_output.stdout);
            for line in output.lines() {
                if !line.is_empty() {
                    projects.push(line.to_string());
                }
            }
        }

        // Find .xcworkspace
        let find_ws_output = Command::new("find")
            .arg(path)
            .args(&depth_args)
            .arg("-name")
            .arg("*.xcworkspace")
            .arg("-type")
            .arg("d")
            .output()
            .await
            .map_err(|e| TestError::Execution(format!("Failed to find workspaces: {}", e)))?;

        if find_ws_output.status.success() {
            let output = String::from_utf8_lossy(&find_ws_output.stdout);
            for line in output.lines() {
                // Filter out xcworkspace inside xcodeproj
                if !line.is_empty() && !line.contains(".xcodeproj/") {
                    workspaces.push(line.to_string());
                }
            }
        }

        Ok(json!({
            "success": true,
            "path": path,
            "projects": projects,
            "workspaces": workspaces,
            "project_count": projects.len(),
            "workspace_count": workspaces.len()
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
    fn test_macos_build_tool_schema() {
        let tool = MacosBuildTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_build");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"build_macos".to_string())
        );
    }

    #[test]
    fn test_macos_build_run_tool_schema() {
        let tool = MacosBuildRunTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_build_run");
    }

    #[test]
    fn test_macos_test_tool_schema() {
        let tool = MacosTestTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_test");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"test_macos".to_string())
        );
    }

    #[test]
    fn test_macos_launch_tool_schema() {
        let tool = MacosLaunchTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_launch");
    }

    #[test]
    fn test_macos_stop_tool_schema() {
        let tool = MacosStopTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_stop");
    }

    #[test]
    fn test_macos_app_path_tool_schema() {
        let tool = MacosAppPathTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_app_path");
    }

    #[test]
    fn test_macos_clean_tool_schema() {
        let tool = MacosCleanTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_clean");
    }

    #[test]
    fn test_macos_bundle_id_tool_schema() {
        let tool = MacosBundleIdTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_bundle_id");
    }

    #[test]
    fn test_macos_build_settings_tool_schema() {
        let tool = MacosBuildSettingsTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_build_settings");
    }

    #[test]
    fn test_macos_list_schemes_tool_schema() {
        let tool = MacosListSchemesTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_list_schemes");
    }

    #[test]
    fn test_macos_discover_projects_tool_schema() {
        let tool = MacosDiscoverProjectsTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "macos_discover_projects");
    }
}
