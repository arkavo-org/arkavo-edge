use super::server::Tool;
use crate::{Result, TestError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::time::sleep;

pub struct TuiTestHarness {
    sessions: Arc<Mutex<HashMap<String, TuiSession>>>,
    schema: ToolSchema,
}

#[derive(Debug)]
struct TuiSession {
    id: String,
    process: Child,
    command: String,
    args: Vec<String>,
    output_buffer: Arc<Mutex<Vec<String>>>,
    _output_thread: thread::JoinHandle<()>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action")]
enum HarnessAction {
    #[serde(rename = "start")]
    Start {
        session_id: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        working_dir: Option<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    #[serde(rename = "stop")]
    Stop { session_id: String },
    #[serde(rename = "send_input")]
    SendInput { session_id: String, input: String },
    #[serde(rename = "get_output")]
    GetOutput {
        session_id: String,
        #[serde(default = "default_lines")]
        last_n_lines: usize,
    },
    #[serde(rename = "list_sessions")]
    ListSessions,
    #[serde(rename = "wait_for_output")]
    WaitForOutput {
        session_id: String,
        expected_text: String,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
}

fn default_lines() -> usize {
    50
}

fn default_timeout() -> u64 {
    5000
}

impl Default for TuiTestHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiTestHarness {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            schema: ToolSchema {
                name: "tui_harness".to_string(),
                aliases: None,
                description: "Manage TUI application sessions for testing".to_string(),
                parameters: json!({
                    "type": "object",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "start"
                                },
                                "session_id": {
                                    "type": "string",
                                    "description": "Unique identifier for the session"
                                },
                                "command": {
                                    "type": "string",
                                    "description": "Command to execute"
                                },
                                "args": {
                                    "type": "array",
                                    "items": {"type": "string"},
                                    "default": []
                                },
                                "working_dir": {
                                    "type": "string",
                                    "description": "Working directory for the command"
                                },
                                "env": {
                                    "type": "object",
                                    "additionalProperties": {"type": "string"},
                                    "default": {}
                                }
                            },
                            "required": ["action", "session_id", "command"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "stop"
                                },
                                "session_id": {
                                    "type": "string"
                                }
                            },
                            "required": ["action", "session_id"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "send_input"
                                },
                                "session_id": {
                                    "type": "string"
                                },
                                "input": {
                                    "type": "string",
                                    "description": "Input to send (including newlines if needed)"
                                }
                            },
                            "required": ["action", "session_id", "input"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "get_output"
                                },
                                "session_id": {
                                    "type": "string"
                                },
                                "last_n_lines": {
                                    "type": "integer",
                                    "default": 50
                                }
                            },
                            "required": ["action", "session_id"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "wait_for_output"
                                },
                                "session_id": {
                                    "type": "string"
                                },
                                "expected_text": {
                                    "type": "string"
                                },
                                "timeout_ms": {
                                    "type": "integer",
                                    "default": 5000
                                }
                            },
                            "required": ["action", "session_id", "expected_text"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "const": "list_sessions"
                                }
                            },
                            "required": ["action"]
                        }
                    ]
                }),
            },
        }
    }

    async fn start_session(
        &self,
        session_id: String,
        command: String,
        args: Vec<String>,
        working_dir: Option<String>,
        env: HashMap<String, String>,
    ) -> Result<()> {
        let mut cmd = Command::new(&command);
        cmd.args(&args);

        // Set up pipes for stdin/stdout/stderr
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Set working directory if specified
        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        // Set environment variables
        for (key, value) in env {
            cmd.env(key, value);
        }

        // Create a new process group to isolate the child (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        // Spawn the process
        let mut process = cmd
            .spawn()
            .map_err(|e| TestError::Mcp(format!("Failed to start process: {e}")))?;

        // Set up output capture
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| TestError::Mcp("Failed to capture stdout".to_string()))?;

        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = output_buffer.clone();

        // Spawn thread to read output
        let output_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(|result| result.ok()) {
                if let Ok(mut buffer) = buffer_clone.lock() {
                    buffer.push(line);
                    // Keep only last 1000 lines to prevent memory issues
                    if buffer.len() > 1000 {
                        buffer.remove(0);
                    }
                }
            }
        });

        // Create session
        let session = TuiSession {
            id: session_id.clone(),
            process,
            command,
            args,
            output_buffer,
            _output_thread: output_thread,
        };

        // Store session - drop lock immediately
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| TestError::Mcp(format!("Failed to lock sessions: {e}")))?;
            sessions.insert(session_id, session);
        }

        Ok(())
    }

    async fn stop_session(&self, session_id: &str) -> Result<()> {
        let session = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| TestError::Mcp(format!("Failed to lock sessions: {e}")))?;
            sessions.remove(session_id)
        };

        if let Some(mut session) = session {
            // Try graceful shutdown first
            if let Some(mut stdin) = session.process.stdin.take() {
                // Send Ctrl+C
                let _ = stdin.write_all(&[3]);
                let _ = stdin.flush();

                // Also try sending 'q' key in case the app uses that for quit
                let _ = stdin.write_all(b"q");
                let _ = stdin.flush();
            }

            // Give it more time to exit gracefully
            sleep(Duration::from_millis(1000)).await;

            // Check if process is still running before force killing
            match session.process.try_wait() {
                Ok(Some(_)) => {
                    // Process already exited
                    eprintln!("[TuiTestHarness] Session {session_id} stopped gracefully");
                }
                Ok(None) => {
                    // Process still running, force kill
                    eprintln!("[TuiTestHarness] Force killing session {session_id}");

                    #[cfg(unix)]
                    {
                        // Kill the entire process group
                        let pid = session.process.id();
                        unsafe {
                            // Kill process group (negative PID)
                            libc::kill(-(pid as i32), libc::SIGTERM);
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            libc::kill(-(pid as i32), libc::SIGKILL);
                        }
                    }

                    #[cfg(not(unix))]
                    {
                        let _ = session.process.kill();
                    }
                }
                Err(e) => {
                    eprintln!("[TuiTestHarness] Error checking process status: {e}");
                    // Try to kill anyway
                    let _ = session.process.kill();
                }
            }

            Ok(())
        } else {
            Err(TestError::Mcp(format!("Session '{session_id}' not found")))
        }
    }

    fn send_input(&self, session_id: &str, input: &str) -> Result<()> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| TestError::Mcp(format!("Failed to lock sessions: {e}")))?;

        if let Some(session) = sessions.get(session_id) {
            if let Some(mut stdin) = session.process.stdin.as_ref() {
                stdin
                    .write_all(input.as_bytes())
                    .map_err(|e| TestError::Mcp(format!("Failed to send input: {e}")))?;
                stdin
                    .flush()
                    .map_err(|e| TestError::Mcp(format!("Failed to flush input: {e}")))?;
                Ok(())
            } else {
                Err(TestError::Mcp("Process stdin not available".to_string()))
            }
        } else {
            Err(TestError::Mcp(format!("Session '{session_id}' not found")))
        }
    }

    fn get_output(&self, session_id: &str, last_n_lines: usize) -> Result<Vec<String>> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| TestError::Mcp(format!("Failed to lock sessions: {e}")))?;

        if let Some(session) = sessions.get(session_id) {
            let buffer = session
                .output_buffer
                .lock()
                .map_err(|e| TestError::Mcp(format!("Failed to lock output buffer: {e}")))?;

            let start = if buffer.len() > last_n_lines {
                buffer.len() - last_n_lines
            } else {
                0
            };

            Ok(buffer[start..].to_vec())
        } else {
            Err(TestError::Mcp(format!("Session '{session_id}' not found")))
        }
    }

    async fn wait_for_output(
        &self,
        session_id: &str,
        expected_text: &str,
        timeout_ms: u64,
    ) -> Result<bool> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            let found = {
                let sessions = self
                    .sessions
                    .lock()
                    .map_err(|e| TestError::Mcp(format!("Failed to lock sessions: {e}")))?;

                if let Some(session) = sessions.get(session_id) {
                    let buffer = session.output_buffer.lock().map_err(|e| {
                        TestError::Mcp(format!("Failed to lock output buffer: {e}"))
                    })?;

                    // Check if any line contains the expected text
                    buffer.iter().any(|line| line.contains(expected_text))
                } else {
                    return Err(TestError::Mcp(format!("Session '{session_id}' not found")));
                }
            }; // Release all locks

            if found {
                return Ok(true);
            }

            sleep(Duration::from_millis(100)).await;
        }

        Ok(false)
    }

    fn list_sessions(&self) -> Result<Vec<HashMap<String, String>>> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| TestError::Mcp(format!("Failed to lock sessions: {e}")))?;

        let session_info: Vec<HashMap<String, String>> = sessions
            .values()
            .map(|session| {
                let mut info = HashMap::new();
                info.insert("id".to_string(), session.id.clone());
                info.insert("command".to_string(), session.command.clone());
                info.insert("args".to_string(), session.args.join(" "));
                info
            })
            .collect();

        Ok(session_info)
    }
}

#[async_trait]
impl Tool for TuiTestHarness {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action: HarnessAction = serde_json::from_value(args)
            .map_err(|e| TestError::Mcp(format!("Invalid parameters: {e}")))?;

        match action {
            HarnessAction::Start {
                session_id,
                command,
                args,
                working_dir,
                env,
            } => {
                self.start_session(session_id.clone(), command, args, working_dir, env)
                    .await?;
                Ok(json!({
                    "success": true,
                    "session_id": session_id,
                    "message": "Session started"
                }))
            }
            HarnessAction::Stop { session_id } => {
                self.stop_session(&session_id).await?;
                Ok(json!({
                    "success": true,
                    "session_id": session_id,
                    "message": "Session stopped"
                }))
            }
            HarnessAction::SendInput { session_id, input } => {
                self.send_input(&session_id, &input)?;
                Ok(json!({
                    "success": true,
                    "session_id": session_id,
                    "message": format!("Sent {} bytes of input", input.len())
                }))
            }
            HarnessAction::GetOutput {
                session_id,
                last_n_lines,
            } => {
                let output = self.get_output(&session_id, last_n_lines)?;
                Ok(json!({
                    "success": true,
                    "session_id": session_id,
                    "output": output,
                    "line_count": output.len()
                }))
            }
            HarnessAction::WaitForOutput {
                session_id,
                expected_text,
                timeout_ms,
            } => {
                let found = self
                    .wait_for_output(&session_id, &expected_text, timeout_ms)
                    .await?;
                Ok(json!({
                    "success": true,
                    "session_id": session_id,
                    "found": found,
                    "expected_text": expected_text
                }))
            }
            HarnessAction::ListSessions => {
                let sessions = self.list_sessions()?;
                Ok(json!({
                    "success": true,
                    "sessions": sessions,
                    "count": sessions.len()
                }))
            }
        }
    }
}

impl Drop for TuiTestHarness {
    fn drop(&mut self) {
        // Clean up any remaining sessions
        if let Ok(sessions) = self.sessions.lock()
            && !sessions.is_empty()
        {
            eprintln!(
                "[TuiTestHarness] Warning: {} sessions still active during drop",
                sessions.len()
            );
            // Note: We don't force kill here as it might affect the parent terminal
            // The processes should be cleaned up by the OS when they lose their parent
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let harness = TuiTestHarness::new();
        let schema = harness.schema();
        assert_eq!(schema.name, "tui_harness");
    }
}
