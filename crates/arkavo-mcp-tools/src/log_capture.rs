use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use uuid::Uuid;

// Global store for active log sessions
lazy_static::lazy_static! {
    static ref LOG_SESSIONS: Arc<Mutex<HashMap<String, LogSession>>> = Arc::new(Mutex::new(HashMap::new()));
}

/// Represents an active log capture session
struct LogSession {
    child: Child,
    logs: Arc<Mutex<Vec<String>>>,
    simulator_id: String,
    bundle_id: Option<String>,
}

// ============================================================================
// LogStartSimTool - Start log capture from simulator
// ============================================================================

pub struct LogStartSimTool {
    schema: ToolSchema,
}

impl LogStartSimTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "log_start_sim".to_string(),
                aliases: Some(vec!["start_sim_log".to_string()]),
                description: "Start capturing logs from an iOS simulator.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "simulator_id": {
                            "type": "string",
                            "description": "Simulator UUID (uses booted if not specified)"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Filter logs by app bundle ID"
                        },
                        "level": {
                            "type": "string",
                            "enum": ["debug", "info", "notice", "error", "fault"],
                            "description": "Minimum log level (default: info)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for LogStartSimTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LogStartSimTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let simulator_id = params
            .get("simulator_id")
            .and_then(|v| v.as_str())
            .unwrap_or("booted")
            .to_string();

        let bundle_id = params.get("bundle_id").and_then(|v| v.as_str()).map(String::from);
        let level = params
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info");

        // Build predicate for filtering
        let mut predicate_parts = vec![format!("messageType >= {}", level)];
        if let Some(ref bid) = bundle_id {
            predicate_parts.push(format!("subsystem CONTAINS '{}'", bid));
        }
        let predicate = predicate_parts.join(" AND ");

        let mut cmd = Command::new("xcrun");
        cmd.args([
            "simctl",
            "spawn",
            &simulator_id,
            "log",
            "stream",
            "--style",
            "compact",
            "--predicate",
            &predicate,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::Execution(format!("Failed to start log capture: {}", e)))?;

        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = Arc::clone(&logs);

        // Spawn task to collect logs
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut logs) = logs_clone.lock() {
                        logs.push(line);
                        // Keep only last 10000 lines to prevent memory issues
                        if logs.len() > 10000 {
                            logs.drain(0..1000);
                        }
                    }
                }
            });
        }

        let session_id = Uuid::new_v4().to_string();

        let session = LogSession {
            child,
            logs,
            simulator_id: simulator_id.clone(),
            bundle_id: bundle_id.clone(),
        };

        if let Ok(mut sessions) = LOG_SESSIONS.lock() {
            sessions.insert(session_id.clone(), session);
        }

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "simulator_id": simulator_id,
            "bundle_id": bundle_id,
            "message": "Log capture started"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// LogStopSimTool - Stop log capture and return logs
// ============================================================================

pub struct LogStopSimTool {
    schema: ToolSchema,
}

impl LogStopSimTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "log_stop_sim".to_string(),
                aliases: Some(vec!["stop_sim_log".to_string()]),
                description: "Stop log capture from simulator and return captured logs.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Log session ID returned by log_start_sim"
                        },
                        "tail": {
                            "type": "integer",
                            "description": "Return only the last N lines (default: all)"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for LogStopSimTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LogStopSimTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("'session_id' is required".to_string()))?;

        let tail = params.get("tail").and_then(|v| v.as_u64()).map(|n| n as usize);

        let mut session = {
            let mut sessions = LOG_SESSIONS
                .lock()
                .map_err(|e| ToolError::Execution(format!("Failed to access sessions: {}", e)))?;
            sessions
                .remove(session_id)
                .ok_or_else(|| ToolError::Execution(format!("Session '{}' not found", session_id)))?
        };

        // Kill the log process
        let _ = session.child.kill().await;

        // Get captured logs
        let logs: Vec<String> = session
            .logs
            .lock()
            .map(|logs| {
                if let Some(n) = tail {
                    logs.iter().rev().take(n).rev().cloned().collect()
                } else {
                    logs.clone()
                }
            })
            .unwrap_or_default();

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "simulator_id": session.simulator_id,
            "bundle_id": session.bundle_id,
            "log_count": logs.len(),
            "logs": logs
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// LogGetSimTool - Get logs from active session without stopping
// ============================================================================

pub struct LogGetSimTool {
    schema: ToolSchema,
}

impl LogGetSimTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "log_get_sim".to_string(),
                aliases: Some(vec!["get_sim_log".to_string()]),
                description: "Get current logs from an active capture session without stopping it.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Log session ID returned by log_start_sim"
                        },
                        "tail": {
                            "type": "integer",
                            "description": "Return only the last N lines (default: 100)"
                        },
                        "clear": {
                            "type": "boolean",
                            "description": "Clear the log buffer after reading (default: false)"
                        }
                    },
                    "required": ["session_id"]
                }),
            },
        }
    }
}

impl Default for LogGetSimTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LogGetSimTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let session_id = params
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("'session_id' is required".to_string()))?;

        let tail = params.get("tail").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
        let clear = params.get("clear").and_then(|v| v.as_bool()).unwrap_or(false);

        let sessions = LOG_SESSIONS
            .lock()
            .map_err(|e| ToolError::Execution(format!("Failed to access sessions: {}", e)))?;

        let session = sessions
            .get(session_id)
            .ok_or_else(|| ToolError::Execution(format!("Session '{}' not found", session_id)))?;

        let logs: Vec<String> = session
            .logs
            .lock()
            .map(|mut logs| {
                let result: Vec<String> = logs.iter().rev().take(tail).rev().cloned().collect();
                if clear {
                    logs.clear();
                }
                result
            })
            .unwrap_or_default();

        Ok(json!({
            "success": true,
            "session_id": session_id,
            "log_count": logs.len(),
            "logs": logs
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// LogListSessionsTool - List active log sessions
// ============================================================================

pub struct LogListSessionsTool {
    schema: ToolSchema,
}

impl LogListSessionsTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "log_list_sessions".to_string(),
                aliases: Some(vec!["list_log_sessions".to_string()]),
                description: "List all active log capture sessions.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        }
    }
}

impl Default for LogListSessionsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LogListSessionsTool {
    async fn execute(&self, _params: Value) -> Result<Value> {
        let sessions = LOG_SESSIONS
            .lock()
            .map_err(|e| ToolError::Execution(format!("Failed to access sessions: {}", e)))?;

        let session_list: Vec<Value> = sessions
            .iter()
            .map(|(id, session)| {
                let log_count = session.logs.lock().map(|l| l.len()).unwrap_or(0);
                json!({
                    "session_id": id,
                    "simulator_id": session.simulator_id,
                    "bundle_id": session.bundle_id,
                    "log_count": log_count
                })
            })
            .collect();

        Ok(json!({
            "success": true,
            "count": session_list.len(),
            "sessions": session_list
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
    fn test_log_start_sim_tool_schema() {
        let tool = LogStartSimTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "log_start_sim");
    }

    #[test]
    fn test_log_stop_sim_tool_schema() {
        let tool = LogStopSimTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "log_stop_sim");
    }

    #[test]
    fn test_log_get_sim_tool_schema() {
        let tool = LogGetSimTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "log_get_sim");
    }

    #[test]
    fn test_log_list_sessions_tool_schema() {
        let tool = LogListSessionsTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "log_list_sessions");
    }
}
