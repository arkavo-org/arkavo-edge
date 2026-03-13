//! Transport trait implementation for subprocess transport

use async_trait::async_trait;
use std::collections::HashMap;
use std::env;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

use crate::error::{ClaudeError, Result};
use crate::utils::truncate_for_display;
use crate::{Transport, VERSION};

use super::{DANGEROUS_ENV_VARS, PromptInput, SubprocessTransport};

#[async_trait]
impl Transport for SubprocessTransport {
    async fn connect(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }

        tracing::debug!(
            "SubprocessTransport::connect - output_format is {:?}",
            self.options.output_format.as_ref().map(|f| &f.format_type)
        );

        let mut cmd = self.build_command()?;

        // Set up environment - strict enforcement of dangerous variable blocking
        let mut process_env = env::vars().collect::<HashMap<_, _>>();

        // Check for dangerous env vars in user-provided options (strict enforcement)
        let dangerous_found: Vec<&String> = self
            .options
            .env
            .keys()
            .filter(|key| DANGEROUS_ENV_VARS.contains(&key.as_str()))
            .collect();

        if !dangerous_found.is_empty() {
            let vars_str = dangerous_found
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            tracing::warn!(
                vars = %vars_str,
                "Rejected dangerous environment variables - possible injection attempt"
            );
            return Err(ClaudeError::invalid_config(format!(
                "Dangerous environment variables detected: [{vars_str}]. These are blocked to prevent injection attacks."
            )));
        }

        // All env vars are safe, add them
        for (key, value) in &self.options.env {
            process_env.insert(key.clone(), value.clone());
        }

        process_env.insert("CLAUDE_CODE_ENTRYPOINT".to_string(), "sdk-rust".to_string());
        process_env.insert("CLAUDE_AGENT_SDK_VERSION".to_string(), VERSION.to_string());

        if let Some(ref cwd) = self.cwd {
            process_env.insert("PWD".to_string(), cwd.to_string_lossy().to_string());
            cmd.current_dir(cwd);
        }

        cmd.envs(process_env);

        // Set up stdio
        // IMPORTANT: We pipe stderr instead of inheriting to prevent the child process
        // from manipulating the parent terminal state. Inheriting stderr gives the child
        // access to the terminal, which can leave it in a corrupted state.
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()); // Pipe stderr to prevent terminal manipulation

        // Log command for debugging
        tracing::debug!("CLI command: {:?}", cmd);

        // Spawn process
        let mut child = cmd.spawn().map_err(|e| {
            if let Some(ref cwd) = self.cwd
                && !cwd.exists()
            {
                #[cfg(debug_assertions)]
                return ClaudeError::connection(format!(
                    "Working directory does not exist: {}",
                    cwd.display()
                ));
                #[cfg(not(debug_assertions))]
                return ClaudeError::connection("Working directory does not exist".to_string());
            }
            ClaudeError::connection(format!("Failed to start Claude Code: {e}"))
        })?;

        // Get stdin, stdout, and stderr
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClaudeError::connection("Failed to get stdin handle"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClaudeError::connection("Failed to get stdout handle"))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ClaudeError::connection("Failed to get stderr handle"))?;

        // Spawn task to consume stderr to prevent blocking
        // We forward it to parent stderr for visibility
        let stderr_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut stderr = stderr;
            let mut buffer = vec![0u8; 4096];

            loop {
                match stderr.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Forward stderr to parent's stderr
                        let _ = std::io::Write::write_all(&mut std::io::stderr(), &buffer[..n]);
                    }
                }
            }
        });

        // Store handles
        self.stdin = Some(stdin);
        self.stdout = Some(BufReader::new(stdout));
        self.process = Some(child);
        self.stderr_task = Some(stderr_task);
        self.ready.store(true, Ordering::SeqCst);

        // For string mode, close stdin immediately
        if matches!(self.prompt, PromptInput::String(_))
            && let Some(mut stdin) = self.stdin.take()
        {
            let _ = stdin.shutdown().await;
        }

        Ok(())
    }

    async fn write(&mut self, data: &str) -> Result<()> {
        if !self.is_ready() {
            return Err(ClaudeError::transport("Transport is not ready for writing"));
        }

        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| ClaudeError::transport("stdin not available"))?;

        stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| ClaudeError::transport(format!("Failed to write to stdin: {e}")))?;

        stdin
            .flush()
            .await
            .map_err(|e| ClaudeError::transport(format!("Failed to flush stdin: {e}")))?;

        Ok(())
    }

    async fn end_input(&mut self) -> Result<()> {
        if let Some(mut stdin) = self.stdin.take() {
            stdin
                .shutdown()
                .await
                .map_err(|e| ClaudeError::transport(format!("Failed to close stdin: {e}")))?;
        }
        Ok(())
    }

    fn read_messages(&mut self) -> mpsc::UnboundedReceiver<Result<serde_json::Value>> {
        let (tx, rx) = mpsc::unbounded_channel();

        // Take ownership of stdout and process
        let stdout = self.stdout.take();
        let process = Arc::new(Mutex::new(self.process.take()));
        let max_buffer_size = self.max_buffer_size;
        let cancel_token = self.cancellation_token.clone();

        // Spawn background task to read messages
        let task = tokio::spawn(async move {
            if stdout.is_none() {
                let _ = tx.send(Err(ClaudeError::connection(
                    "Not connected - stdout not available",
                )));
                return;
            }

            let mut stdout = stdout.unwrap();
            let mut json_buffer = String::new();

            loop {
                let mut line = String::new();

                // Use select! to allow cancellation - no hardcoded timeout
                // The caller controls cancellation via the CancellationToken
                tokio::select! {
                    // Check for cancellation
                    () = cancel_token.cancelled() => {
                        tracing::debug!("Read cancelled via CancellationToken");
                        break;
                    }
                    // Read next line
                    result = stdout.read_line(&mut line) => {
                        match result {
                            Ok(0) => break, // EOF
                            Ok(_) => {
                                let line = line.trim();
                                if line.is_empty() {
                                    continue;
                                }

                                // Accumulate partial JSON until we can parse it
                                json_buffer.push_str(line);

                                if json_buffer.len() > max_buffer_size {
                                    // Safe truncation for error preview (respects UTF-8 boundaries)
                                    let preview = truncate_for_display(&json_buffer, 100);
                                    let _ = tx.send(Err(ClaudeError::JsonDecode(
                                        serde_json::Error::io(std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            format!(
                                                "JSON message exceeded maximum buffer size of {max_buffer_size} bytes. Preview: {preview}"
                                            ),
                                        )),
                                    )));
                                    json_buffer.clear();
                                    continue;
                                }

                                // Try to parse JSON
                                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&json_buffer) {
                                    let msg_type = data.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
                                    tracing::trace!(msg_type, "Received message from CLI");

                                    // Debug log for result messages
                                    if msg_type == "result" {
                                        tracing::debug!(
                                            "Raw result JSON: {}",
                                            json_buffer.chars().take(500).collect::<String>()
                                        );
                                    }

                                    json_buffer.clear();
                                    if tx.send(Ok(data)).is_err() {
                                        // Receiver dropped, stop reading
                                        break;
                                    }
                                }
                                // else: Not complete yet, continue accumulating
                            }
                            Err(e) => {
                                let _ = tx.send(Err(ClaudeError::Io(e)));
                                break;
                            }
                        }
                    }
                }
            }

            // Check process exit code
            if let Ok(mut process_guard) = process.try_lock()
                && let Some(mut child) = process_guard.take()
            {
                match child.wait().await {
                    Ok(status) => {
                        if !status.success()
                            && let Some(code) = status.code()
                        {
                            let _ = tx.send(Err(ClaudeError::process(
                                "Command failed",
                                code,
                                Some("Check stderr output for details".to_string()),
                            )));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(ClaudeError::Io(e)));
                    }
                }
            }
        });

        // Store task handle for cleanup
        self.reader_task = Some(task);

        rx
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    async fn close(&mut self) -> Result<()> {
        self.ready.store(false, Ordering::SeqCst);

        // Cancel any ongoing read operations via token
        self.cancellation_token.cancel();

        // Close stdin to signal the process to exit gracefully
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.shutdown().await;
        }

        // Wait for reader task to finish (it will exit due to cancellation)
        if let Some(task) = self.reader_task.take() {
            // Give a brief window for graceful exit before abort
            tokio::select! {
                _ = task => {}
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        self.stdout = None;

        // Try to wait for the process to exit gracefully first
        if let Some(mut child) = self.process.take() {
            // Give the process a configurable timeout to exit gracefully
            let timeout_duration = std::time::Duration::from_secs(5);

            match tokio::time::timeout(timeout_duration, child.wait()).await {
                Ok(Ok(_status)) => {
                    // Process exited gracefully
                }
                Ok(Err(e)) => {
                    return Err(ClaudeError::Io(e));
                }
                Err(_) => {
                    // Timeout - kill the process
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
        }

        Ok(())
    }
}
