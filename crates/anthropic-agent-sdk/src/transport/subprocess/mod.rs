//! Subprocess transport implementation using Claude Code CLI

mod command_builder;
mod transport_impl;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::error::{ClaudeError, Result};
use crate::types::ClaudeAgentOptions;

pub(super) const DEFAULT_MAX_BUFFER_SIZE: usize = 1024 * 1024; // 1MB

// Dangerous environment variables that should not be passed to subprocess
pub(super) const DANGEROUS_ENV_VARS: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "PATH",
    "NODE_OPTIONS",
    "PYTHONPATH",
    "PERL5LIB",
    "RUBYLIB",
];

// Allowed extra CLI flags (allowlist approach)
pub(super) const ALLOWED_EXTRA_FLAGS: &[&str] = &["timeout", "retries", "log-level", "cache-dir"];

/// Prompt input type
#[derive(Debug)]
pub enum PromptInput {
    /// Single string prompt
    String(String),
    /// Stream of JSON messages
    Stream,
}

impl From<String> for PromptInput {
    fn from(s: String) -> Self {
        PromptInput::String(s)
    }
}

impl From<&str> for PromptInput {
    fn from(s: &str) -> Self {
        PromptInput::String(s.to_string())
    }
}

/// Subprocess transport for Claude Code CLI
pub struct SubprocessTransport {
    pub(super) prompt: PromptInput,
    pub(super) options: ClaudeAgentOptions,
    pub(super) cli_path: PathBuf,
    pub(super) cwd: Option<PathBuf>,
    pub(super) process: Option<Child>,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) stdout: Option<BufReader<ChildStdout>>,
    pub(super) ready: Arc<AtomicBool>,
    pub(super) max_buffer_size: usize,
    pub(super) reader_task: Option<JoinHandle<()>>,
    pub(super) stderr_task: Option<JoinHandle<()>>,
    /// Cancellation token for aborting operations (like `AbortController` in JS)
    pub(super) cancellation_token: CancellationToken,
}

impl SubprocessTransport {
    /// Create a new subprocess transport
    ///
    /// # Arguments
    /// * `prompt` - The prompt input (string or stream)
    /// * `options` - Configuration options
    /// * `cli_path` - Optional path to Claude Code CLI (will search if None)
    ///
    /// # Errors
    /// Returns error if CLI cannot be found
    pub fn new(
        prompt: PromptInput,
        options: ClaudeAgentOptions,
        cli_path: Option<PathBuf>,
    ) -> Result<Self> {
        Self::with_cancellation_token(prompt, options, cli_path, None)
    }

    /// Create a new subprocess transport with an optional parent cancellation token
    ///
    /// # Arguments
    /// * `prompt` - The prompt input (string or stream)
    /// * `options` - Configuration options
    /// * `cli_path` - Optional path to Claude Code CLI (will search if None)
    /// * `cancellation_token` - Optional parent cancellation token from client
    ///
    /// # Errors
    /// Returns error if CLI cannot be found
    pub fn with_cancellation_token(
        prompt: PromptInput,
        options: ClaudeAgentOptions,
        cli_path: Option<PathBuf>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<Self> {
        tracing::debug!(
            "SubprocessTransport::new - output_format: {:?}",
            options.output_format.as_ref().map(|f| &f.format_type)
        );

        let cli_path = if let Some(path) = cli_path {
            path
        } else {
            Self::find_cli()?
        };

        let cwd = options.cwd.clone();
        let max_buffer_size = options.max_buffer_size.unwrap_or(DEFAULT_MAX_BUFFER_SIZE);

        // Use provided token or create a new one
        let token = cancellation_token.unwrap_or_default();

        Ok(Self {
            prompt,
            options,
            cli_path,
            cwd,
            process: None,
            stdin: None,
            stdout: None,
            ready: Arc::new(AtomicBool::new(false)),
            max_buffer_size,
            reader_task: None,
            stderr_task: None,
            cancellation_token: token,
        })
    }

    /// Find Claude Code CLI binary
    pub(super) fn find_cli() -> Result<PathBuf> {
        // Try using 'which' crate first
        if let Ok(path) = which::which("claude") {
            return Ok(path);
        }

        // Manual search in common locations
        let home = env::var("HOME").unwrap_or_else(|_| String::from("/root"));
        let locations = vec![
            PathBuf::from(home.clone()).join(".npm-global/bin/claude"),
            PathBuf::from("/usr/local/bin/claude"),
            PathBuf::from(home.clone()).join(".local/bin/claude"),
            PathBuf::from(home.clone()).join("node_modules/.bin/claude"),
            PathBuf::from(home).join(".yarn/bin/claude"),
        ];

        for path in locations {
            if path.exists() && path.is_file() {
                return Ok(path);
            }
        }

        Err(ClaudeError::cli_not_found())
    }

    /// Get a child cancellation token for this transport
    /// Callers can use this to cancel ongoing operations
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.child_token()
    }

    /// Cancel all ongoing operations
    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    /// Serialize MCP config for CLI
    pub(super) fn serialize_mcp_config(
        config: &crate::types::McpServerConfig,
    ) -> serde_json::Value {
        match config {
            crate::types::McpServerConfig::Stdio(stdio) => {
                let mut obj = serde_json::json!({
                    "command": stdio.command,
                });
                if let Some(ref args) = stdio.args {
                    obj["args"] = serde_json::json!(args);
                }
                if let Some(ref env) = stdio.env {
                    obj["env"] = serde_json::json!(env);
                }
                if let Some(ref server_type) = stdio.server_type {
                    obj["type"] = serde_json::json!(server_type);
                }
                obj
            }
            crate::types::McpServerConfig::Sse(sse) => {
                serde_json::json!({
                    "type": sse.server_type,
                    "url": sse.url,
                    "headers": sse.headers,
                })
            }
            crate::types::McpServerConfig::Http(http) => {
                serde_json::json!({
                    "type": http.server_type,
                    "url": http.url,
                    "headers": http.headers,
                })
            }
            crate::types::McpServerConfig::Sdk(sdk) => {
                let mut obj = serde_json::json!({
                    "type": "sdk",
                    "name": sdk.name,
                });
                if let Some(ref version) = sdk.version {
                    obj["version"] = serde_json::json!(version);
                }
                obj
            }
        }
    }
}

impl Drop for SubprocessTransport {
    fn drop(&mut self) {
        // Close stdin if still open to signal graceful shutdown
        if let Some(stdin) = self.stdin.take() {
            // Drop will close it
            drop(stdin);
        }

        // Abort reader task if running
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }

        // Abort stderr task if running
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        // Try graceful shutdown first, then kill if needed
        if let Some(mut child) = self.process.take() {
            // Try to kill gracefully (SIGTERM on Unix)
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Transport;

    #[test]
    fn test_find_cli() {
        // This will succeed if claude is installed
        let result = SubprocessTransport::find_cli();
        // We can't assert success because it depends on environment
        println!("CLI search result: {result:?}");
    }

    #[test]
    fn test_prompt_input_conversions() {
        let _prompt1: PromptInput = "hello".into();
        let _prompt2: PromptInput = String::from("world").into();
    }

    #[test]
    fn test_extra_args_allowlist_rejects_disallowed() {
        // Use same CLI discovery as production code
        let Ok(cli_path) = SubprocessTransport::find_cli() else {
            return; // Skip if CLI not installed
        };

        let mut options = ClaudeAgentOptions::default();
        options
            .extra_args
            .insert("dangerous-flag".to_string(), None);

        let transport = SubprocessTransport::new(PromptInput::Stream, options, Some(cli_path))
            .expect("Transport creation should succeed");

        let result = transport.build_command();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Disallowed CLI flags"));
        assert!(err.to_string().contains("dangerous-flag"));
    }

    #[test]
    fn test_extra_args_allowlist_accepts_allowed() {
        // Use same CLI discovery as production code
        let Ok(cli_path) = SubprocessTransport::find_cli() else {
            return; // Skip if CLI not installed
        };

        let mut options = ClaudeAgentOptions::default();
        options
            .extra_args
            .insert("timeout".to_string(), Some("30".to_string()));
        options
            .extra_args
            .insert("log-level".to_string(), Some("debug".to_string()));

        let transport = SubprocessTransport::new(PromptInput::Stream, options, Some(cli_path))
            .expect("Transport creation should succeed");

        let result = transport.build_command();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dangerous_env_vars_rejected() {
        // Use same CLI discovery as production code
        let Ok(cli_path) = SubprocessTransport::find_cli() else {
            return; // Skip if CLI not installed
        };

        let mut options = ClaudeAgentOptions::default();
        options
            .env
            .insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());

        let mut transport = SubprocessTransport::new(PromptInput::Stream, options, Some(cli_path))
            .expect("Transport creation should succeed");

        let result = transport.connect().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Dangerous environment variables"));
        assert!(err.to_string().contains("LD_PRELOAD"));
    }

    #[tokio::test]
    async fn test_safe_env_vars_accepted() {
        // Use same CLI discovery as production code
        let Ok(cli_path) = SubprocessTransport::find_cli() else {
            return; // Skip if CLI not installed
        };

        let mut options = ClaudeAgentOptions::default();
        options
            .env
            .insert("MY_SAFE_VAR".to_string(), "safe value".to_string());

        let mut transport = SubprocessTransport::new(PromptInput::Stream, options, Some(cli_path))
            .expect("Transport creation should succeed");

        // Connect should succeed (env vars are safe)
        let result = transport.connect().await;
        assert!(result.is_ok());

        // Cleanup
        transport.close().await.ok();
    }

    #[test]
    fn test_session_id_option_maps_to_cli_arg() {
        let Ok(cli_path) = SubprocessTransport::find_cli() else {
            return; // Skip if CLI not installed
        };

        let options = ClaudeAgentOptions::builder()
            .session_id("550e8400-e29b-41d4-a716-446655440000")
            .build();

        let transport = SubprocessTransport::new(PromptInput::Stream, options, Some(cli_path))
            .expect("Transport creation should succeed");

        let cmd = transport
            .build_command()
            .expect("build_command should succeed");
        // Convert tokio Command to std Command to access get_args()
        let std_cmd = cmd.as_std();
        let args: Vec<String> = std_cmd
            .get_args()
            .filter_map(|a| a.to_str().map(String::from))
            .collect();

        // Find --session-id flag
        let session_id_idx = args.iter().position(|a| a == "--session-id");
        assert!(
            session_id_idx.is_some(),
            "Expected --session-id flag in args"
        );

        // Verify the value follows the flag
        let value = &args[session_id_idx.unwrap() + 1];
        assert_eq!(value, "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn test_plugin_dir_flag_correct() {
        let Ok(cli_path) = SubprocessTransport::find_cli() else {
            return; // Skip if CLI not installed
        };

        let options = ClaudeAgentOptions::builder()
            .plugins(vec![crate::types::SdkPluginConfig::Local {
                path: "/path/to/plugin".to_string(),
            }])
            .build();

        let transport = SubprocessTransport::new(PromptInput::Stream, options, Some(cli_path))
            .expect("Transport creation should succeed");

        let cmd = transport
            .build_command()
            .expect("build_command should succeed");
        // Convert tokio Command to std Command to access get_args()
        let std_cmd = cmd.as_std();
        let args: Vec<String> = std_cmd
            .get_args()
            .filter_map(|a| a.to_str().map(String::from))
            .collect();

        // Should use --plugin-dir, not --plugin
        let plugin_dir_idx = args.iter().position(|a| a == "--plugin-dir");
        assert!(
            plugin_dir_idx.is_some(),
            "Expected --plugin-dir flag in args, got: {args:?}"
        );

        // Verify the value follows the flag
        let value = &args[plugin_dir_idx.unwrap() + 1];
        assert_eq!(value, "/path/to/plugin");
    }
}
