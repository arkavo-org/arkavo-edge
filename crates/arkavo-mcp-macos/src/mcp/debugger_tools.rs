use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

// Global store for active debug sessions using tokio Mutex
static DEBUG_SESSIONS: std::sync::LazyLock<Arc<Mutex<HashMap<String, DebugSession>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Represents an active LLDB debug session
struct DebugSession {
    child: Child,
    stdin: Option<tokio::process::ChildStdin>,
    output_buffer: Arc<std::sync::Mutex<Vec<String>>>,
}

/// Breakpoint information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakpointInfo {
    pub id: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub function: Option<String>,
    pub condition: Option<String>,
    pub enabled: bool,
}

// ============================================================================
// DebugAttachTool - Attach LLDB to a process
// ============================================================================

pub struct DebugAttachTool {
    schema: ToolSchema,
}

impl DebugAttachTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_attach".to_string(),
                aliases: Some(vec!["lldb_attach".to_string()]),
                description: "Attach LLDB debugger to a running process.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pid": {
                            "type": "integer",
                            "description": "Process ID to attach to"
                        },
                        "name": {
                            "type": "string",
                            "description": "Process name to attach to"
                        },
                        "wait_for": {
                            "type": "boolean",
                            "description": "Wait for process to launch (default: false)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for DebugAttachTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugAttachTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let pid = params.get("pid").and_then(|v| v.as_i64());
        let name = params.get("name").and_then(|v| v.as_str());
        let wait_for = params
            .get("wait_for")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if pid.is_none() && name.is_none() {
            return Err(TestError::Mcp(
                "Either 'pid' or 'name' is required".to_string(),
            ));
        }

        let mut cmd = Command::new("lldb");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let target_info = if let Some(p) = pid {
            cmd.args(["-p", &p.to_string()]);
            format!("pid:{}", p)
        } else if let Some(n) = name {
            if wait_for {
                cmd.args(["-n", n, "-w"]);
            } else {
                cmd.args(["-n", n]);
            }
            format!("name:{}", n)
        } else {
            return Err(TestError::Mcp("No target specified".to_string()));
        };

        let mut child = cmd
            .spawn()
            .map_err(|e| TestError::Execution(format!("Failed to start LLDB: {}", e)))?;

        let output_buffer = Arc::new(std::sync::Mutex::new(Vec::new()));
        let output_clone = Arc::clone(&output_buffer);

        // Spawn task to collect output
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut buf) = output_clone.lock() {
                        buf.push(line);
                        if buf.len() > 1000 {
                            buf.drain(0..100);
                        }
                    }
                }
            });
        }

        let stdin = child.stdin.take();
        let session_id = Uuid::new_v4().to_string();

        let session = DebugSession {
            child,
            stdin,
            output_buffer,
        };

        {
            let mut sessions = DEBUG_SESSIONS.lock().await;
            sessions.insert(session_id.clone(), session);
        }

        // Wait briefly for attachment
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "target": target_info,
            "message": "Debugger attached"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DebugBreakpointAddTool - Add a breakpoint
// ============================================================================

pub struct DebugBreakpointAddTool {
    schema: ToolSchema,
}

impl DebugBreakpointAddTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_breakpoint_add".to_string(),
                aliases: Some(vec!["add_breakpoint".to_string(), "bp".to_string()]),
                description: "Add a breakpoint in the debugger.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Debug session ID"
                        },
                        "file": {
                            "type": "string",
                            "description": "Source file path"
                        },
                        "line": {
                            "type": "integer",
                            "description": "Line number"
                        },
                        "function": {
                            "type": "string",
                            "description": "Function name"
                        },
                        "condition": {
                            "type": "string",
                            "description": "Breakpoint condition expression"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for DebugBreakpointAddTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugBreakpointAddTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        let file = params.get("file").and_then(|v| v.as_str());
        let line = params.get("line").and_then(|v| v.as_u64());
        let function = params.get("function").and_then(|v| v.as_str());
        let condition = params.get("condition").and_then(|v| v.as_str());

        let command = if let (Some(f), Some(l)) = (file, line) {
            format!("breakpoint set --file {} --line {}", f, l)
        } else if let Some(func) = function {
            format!("breakpoint set --name {}", func)
        } else {
            return Err(TestError::Mcp(
                "Either 'file' and 'line', or 'function' is required".to_string(),
            ));
        };

        let output = send_lldb_command(session_id, &command).await?;

        // Add condition if specified
        if let Some(cond) = condition
            && output.contains("Breakpoint")
            && let Some(bp_num) = extract_breakpoint_number(&output)
        {
            let cond_cmd = format!("breakpoint modify {} --condition '{}'", bp_num, cond);
            send_lldb_command(session_id, &cond_cmd).await?;
        }

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "output": output,
            "message": "Breakpoint added"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DebugContinueTool - Continue execution
// ============================================================================

pub struct DebugContinueTool {
    schema: ToolSchema,
}

impl DebugContinueTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_continue".to_string(),
                aliases: Some(vec!["continue".to_string(), "resume".to_string()]),
                description: "Continue execution in the debugger.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Debug session ID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for DebugContinueTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugContinueTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        let output = send_lldb_command(session_id, "continue").await?;

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DebugStepTool - Step execution
// ============================================================================

pub struct DebugStepTool {
    schema: ToolSchema,
}

impl DebugStepTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_step".to_string(),
                aliases: Some(vec!["step".to_string()]),
                description: "Step execution (over, into, or out).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Debug session ID"
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["over", "into", "out"],
                            "description": "Step mode (default: over)"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for DebugStepTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugStepTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        let mode = params
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("over");

        let command = match mode {
            "into" => "step",
            "out" => "finish",
            _ => "next",
        };

        let output = send_lldb_command(session_id, command).await?;

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "mode": mode,
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DebugStackTool - Get call stack
// ============================================================================

pub struct DebugStackTool {
    schema: ToolSchema,
}

impl DebugStackTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_stack".to_string(),
                aliases: Some(vec!["backtrace".to_string(), "bt".to_string()]),
                description: "Get the call stack (backtrace).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Debug session ID"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum frames to show (default: all)"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for DebugStackTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugStackTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        let limit = params.get("limit").and_then(|v| v.as_u64());

        let command = if let Some(l) = limit {
            format!("thread backtrace -c {}", l)
        } else {
            "thread backtrace".to_string()
        };

        let output = send_lldb_command(session_id, &command).await?;

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "stack": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DebugVariablesTool - Inspect variables
// ============================================================================

pub struct DebugVariablesTool {
    schema: ToolSchema,
}

impl DebugVariablesTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_variables".to_string(),
                aliases: Some(vec!["vars".to_string(), "locals".to_string()]),
                description: "Inspect variables in the current frame.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Debug session ID"
                        },
                        "expression": {
                            "type": "string",
                            "description": "Specific expression to evaluate"
                        },
                        "scope": {
                            "type": "string",
                            "enum": ["locals", "args", "globals", "all"],
                            "description": "Variable scope (default: locals)"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for DebugVariablesTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugVariablesTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        let expression = params.get("expression").and_then(|v| v.as_str());
        let scope = params
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("locals");

        let command = if let Some(expr) = expression {
            format!("expression {}", expr)
        } else {
            match scope {
                "args" => "frame variable --no-args false --no-locals true".to_string(),
                "globals" => "target variable".to_string(),
                "all" => "frame variable".to_string(),
                _ => "frame variable --no-args true".to_string(),
            }
        };

        let output = send_lldb_command(session_id, &command).await?;

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "scope": scope,
            "variables": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DebugDetachTool - Detach from process
// ============================================================================

pub struct DebugDetachTool {
    schema: ToolSchema,
}

impl DebugDetachTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_detach".to_string(),
                aliases: Some(vec!["detach".to_string()]),
                description: "Detach the debugger from the process.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Debug session ID"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for DebugDetachTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugDetachTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        // Send quit command
        let _ = send_lldb_command(session_id, "detach").await;
        let _ = send_lldb_command(session_id, "quit").await;

        // Remove session
        let mut sessions = DEBUG_SESSIONS.lock().await;
        if let Some(mut session) = sessions.remove(session_id) {
            let _ = session.child.kill().await;
        }

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "message": "Debugger detached"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DebugCommandTool - Send raw LLDB command
// ============================================================================

pub struct DebugCommandTool {
    schema: ToolSchema,
}

impl DebugCommandTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "debug_command".to_string(),
                aliases: Some(vec!["lldb".to_string()]),
                description: "Send a raw LLDB command.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Debug session ID"
                        },
                        "command": {
                            "type": "string",
                            "description": "LLDB command to execute"
                        }
                    },
                    "required": ["session_id", "command"]
                }),
            },
        }
    }
}

impl Default for DebugCommandTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DebugCommandTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'session_id' is required".to_string()))?;

        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("'command' is required".to_string()))?;

        let output = send_lldb_command(session_id, command).await?;

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "command": command,
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// Helper function to send command to LLDB session
async fn send_lldb_command(session_id: &str, command: &str) -> Result<String> {
    // Get stdin and write command
    {
        let mut sessions = DEBUG_SESSIONS.lock().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| TestError::Execution(format!("Session '{}' not found", session_id)))?;

        if let Some(ref mut stdin) = session.stdin {
            stdin
                .write_all(format!("{}\n", command).as_bytes())
                .await
                .map_err(|e| TestError::Execution(format!("Failed to send command: {}", e)))?;
            stdin.flush().await.ok();
        }
    }

    // Wait briefly for output
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Get output
    let sessions = DEBUG_SESSIONS.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| TestError::Execution(format!("Session '{}' not found", session_id)))?;

    let output = session
        .output_buffer
        .lock()
        .map(|buf| buf.join("\n"))
        .unwrap_or_default();

    Ok(output)
}

// Helper to extract breakpoint number from LLDB output
fn extract_breakpoint_number(output: &str) -> Option<u32> {
    for line in output.lines() {
        if let Some(num_str) = line.strip_prefix("Breakpoint ")
            && let Some(num_end) = num_str.find(':')
            && let Ok(num) = num_str[..num_end].parse()
        {
            return Some(num);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_attach_tool_schema() {
        let tool = DebugAttachTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_attach");
    }

    #[test]
    fn test_debug_breakpoint_add_tool_schema() {
        let tool = DebugBreakpointAddTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_breakpoint_add");
        assert!(schema.aliases.as_ref().unwrap().contains(&"bp".to_string()));
    }

    #[test]
    fn test_debug_continue_tool_schema() {
        let tool = DebugContinueTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_continue");
    }

    #[test]
    fn test_debug_step_tool_schema() {
        let tool = DebugStepTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_step");
    }

    #[test]
    fn test_debug_stack_tool_schema() {
        let tool = DebugStackTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_stack");
        assert!(schema.aliases.as_ref().unwrap().contains(&"bt".to_string()));
    }

    #[test]
    fn test_debug_variables_tool_schema() {
        let tool = DebugVariablesTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_variables");
    }

    #[test]
    fn test_debug_detach_tool_schema() {
        let tool = DebugDetachTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_detach");
    }

    #[test]
    fn test_debug_command_tool_schema() {
        let tool = DebugCommandTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "debug_command");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"lldb".to_string())
        );
    }

    #[test]
    fn test_extract_breakpoint_number() {
        let output = "Breakpoint 3: where = MyApp`main, address = 0x0000000100001234";
        assert_eq!(extract_breakpoint_number(output), Some(3));

        let output = "No breakpoint found";
        assert_eq!(extract_breakpoint_number(output), None);
    }
}
