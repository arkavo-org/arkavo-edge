//! Interactive chat commands for A2A sessions
//!
//! Provides testable command execution that integrates with the A2A protocol.
//! Commands can be executed programmatically by agent coders or via CLI.

use crate::a2a::{A2aClient, A2aClientError};
use crate::types::{MessageDelta, MessageDeltaContent};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::Router;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Repository context attachment mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextMode {
    /// Never attach repository context
    Off,
    /// Always attach repository context
    On,
    /// Automatically detect when context is needed
    #[default]
    Auto,
}

impl std::str::FromStr for ContextMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" | "false" | "0" => Ok(Self::Off),
            "on" | "true" | "1" => Ok(Self::On),
            "auto" => Ok(Self::Auto),
            _ => Err(format!("Invalid context mode: {s}. Use: off, on, auto")),
        }
    }
}

impl std::fmt::Display for ContextMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::On => write!(f, "on"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

/// Result of executing a chat command
#[derive(Debug, Clone)]
pub enum CommandResult {
    /// Command executed successfully with optional message
    Success(Option<String>),
    /// Command produced output (e.g., file listing, history)
    Output(String),
    /// Command requests exit from the session
    Exit,
    /// Command failed with error message
    Error(String),
}

/// File content to be included in the next message
#[derive(Debug, Clone)]
pub struct PendingContext {
    /// File path
    pub path: String,
    /// File content
    pub content: String,
}

/// Interactive chat session with command support
///
/// Wraps the A2A client and provides command execution that can be:
/// - Called programmatically in tests
/// - Used by agent coders
/// - Driven by CLI input
pub struct ChatSession {
    client: A2aClient,
    context_mode: ContextMode,
    pending_context: Vec<PendingContext>,
    message_history: Vec<(String, String)>, // (role, content)
}

impl ChatSession {
    /// Create a new chat session with router and optional tool registry
    pub async fn new(
        router: Arc<Router>,
        tool_registry: Option<Arc<ToolRegistry>>,
    ) -> Result<Self, A2aClientError> {
        Self::new_with_model(router, tool_registry, None).await
    }

    /// Create a new chat session with an optional model override for testing
    pub async fn new_with_model(
        router: Arc<Router>,
        tool_registry: Option<Arc<ToolRegistry>>,
        model_name: Option<&str>,
    ) -> Result<Self, A2aClientError> {
        let mut client = A2aClient::with_router_and_model(router, tool_registry, model_name);
        client.open_session().await?;

        Ok(Self {
            client,
            context_mode: ContextMode::default(),
            pending_context: Vec::new(),
            message_history: Vec::new(),
        })
    }

    /// Create a chat session from an existing A2A client
    pub fn from_client(client: A2aClient) -> Self {
        Self {
            client,
            context_mode: ContextMode::default(),
            pending_context: Vec::new(),
            message_history: Vec::new(),
        }
    }

    /// Get the current session ID
    pub fn session_id(&self) -> Option<&str> {
        self.client.session_id()
    }

    /// Check if the session is active
    pub fn is_active(&self) -> bool {
        self.client.has_session()
    }

    /// Get current context mode
    pub fn context_mode(&self) -> ContextMode {
        self.context_mode
    }

    /// Get pending context files
    pub fn pending_context(&self) -> &[PendingContext] {
        &self.pending_context
    }

    /// Get message history
    pub fn history(&self) -> &[(String, String)] {
        &self.message_history
    }

    /// Execute /new command - start a new session
    pub async fn cmd_new(&mut self) -> CommandResult {
        if let Err(e) = self.client.close_session().await {
            // Log but don't fail - session might already be closed
            tracing::warn!("Error closing previous session: {e}");
        }

        match self.client.open_session().await {
            Ok(session_id) => {
                self.pending_context.clear();
                self.message_history.clear();
                let short_id = &session_id[..8.min(session_id.len())];
                CommandResult::Success(Some(format!("Started new session: {short_id}")))
            }
            Err(e) => CommandResult::Error(format!("Failed to start new session: {e}")),
        }
    }

    /// Execute /clear command - clear conversation context
    pub fn cmd_clear(&mut self) -> CommandResult {
        self.pending_context.clear();
        self.message_history.clear();
        // Note: The actual conversation context is maintained by ChatSessionManager
        // A full clear would require closing and reopening the session
        CommandResult::Success(Some("Context cleared".to_string()))
    }

    /// Execute /context command - set context mode
    pub fn cmd_context(&mut self, mode: &str) -> CommandResult {
        if mode.is_empty() {
            return CommandResult::Output(format!("Current context mode: {}", self.context_mode));
        }

        match mode.parse::<ContextMode>() {
            Ok(new_mode) => {
                self.context_mode = new_mode;
                CommandResult::Success(Some(format!("Context mode set to: {new_mode}")))
            }
            Err(e) => CommandResult::Error(e),
        }
    }

    /// Execute /history command - show conversation history
    pub fn cmd_history(&self) -> CommandResult {
        use std::fmt::Write;

        if self.message_history.is_empty() {
            return CommandResult::Output("No messages in history".to_string());
        }

        let mut output = String::from("Conversation history:\n");
        for (i, (role, content)) in self.message_history.iter().enumerate() {
            let preview = if content.len() > 100 {
                format!("{}...", &content[..100])
            } else {
                content.clone()
            };
            let _ = writeln!(output, "\n{}. [{role}]: {preview}", i + 1);
        }

        CommandResult::Output(output)
    }

    /// Execute /read command - read file into pending context
    pub fn cmd_read(&mut self, path: &str) -> CommandResult {
        let file_path = Path::new(path);

        match std::fs::read_to_string(file_path) {
            Ok(content) => {
                let file_size = content.len();
                let preview = if content.len() > 200 {
                    format!("{}...", &content[..200])
                } else {
                    content.clone()
                };

                self.pending_context.push(PendingContext {
                    path: path.to_string(),
                    content,
                });

                CommandResult::Output(format!(
                    "Read {path} ({file_size} bytes, will be included in next message):\n{preview}"
                ))
            }
            Err(e) => CommandResult::Error(format!("Failed to read {path}: {e}")),
        }
    }

    /// Execute /list command - list directory contents
    pub fn cmd_list(&self, dir: Option<&str>) -> CommandResult {
        use std::fmt::Write;

        let path = dir.unwrap_or(".");

        match std::fs::read_dir(path) {
            Ok(entries) => {
                let mut output = format!("Contents of {path}:\n");
                let mut items: Vec<_> = entries.flatten().collect();
                items.sort_by_key(|e| e.file_name());

                for entry in items {
                    let file_type = if entry.path().is_dir() { "d" } else { "-" };
                    let name = entry.file_name().to_string_lossy().to_string();
                    let _ = writeln!(output, "  {file_type} {name}");
                }

                CommandResult::Output(output)
            }
            Err(e) => CommandResult::Error(format!("Failed to list {path}: {e}")),
        }
    }

    /// Execute /exit command - close session and exit
    pub async fn cmd_exit(&mut self) -> CommandResult {
        if let Err(e) = self.client.close_session().await {
            tracing::warn!("Error closing session on exit: {e}");
        }
        CommandResult::Exit
    }

    /// Execute /help command - show available commands
    pub fn cmd_help(&self) -> CommandResult {
        CommandResult::Output(
            r#"
Interactive Commands:
  /new              Start a new conversation
  /clear            Clear conversation context
  /context [mode]   Get/set repo context mode (off, on, auto)
  /history          Show conversation history
  /read <file>      Read file into context (included in next message)
  /list [dir]       List directory contents
  /exit, /quit, /q  Exit chat
  /help, /?         Show this help
"#
            .to_string(),
        )
    }

    /// Send a message to the LLM and get streaming response
    ///
    /// If there is pending context from /read commands, it will be prepended.
    pub async fn send_message(
        &mut self,
        content: &str,
    ) -> Result<mpsc::Receiver<MessageDelta>, A2aClientError> {
        use std::fmt::Write;

        // Build message with pending context
        let full_message = if self.pending_context.is_empty() {
            content.to_string()
        } else {
            let mut msg = String::new();
            for ctx in &self.pending_context {
                let _ = writeln!(msg, "File: {}\n```\n{}\n```\n", ctx.path, ctx.content);
            }
            msg.push_str(content);
            self.pending_context.clear();
            msg
        };

        // Record in history
        self.message_history
            .push(("user".to_string(), content.to_string()));

        // Send via A2A client
        self.client.send_message(&full_message).await
    }

    /// Collect streaming response into a complete string
    ///
    /// Records the response in history.
    pub async fn collect_response(
        &mut self,
        rx: &mut mpsc::Receiver<MessageDelta>,
    ) -> Result<String, A2aClientError> {
        let mut response = String::new();

        while let Some(delta) = rx.recv().await {
            match &delta.delta {
                MessageDeltaContent::Text { text } => {
                    response.push_str(text);
                }
                MessageDeltaContent::StreamEnd { .. } => {
                    break;
                }
                MessageDeltaContent::Error { message, .. } => {
                    return Err(A2aClientError::SendFailed(message.clone()));
                }
                _ => {}
            }
        }

        // Record in history
        if !response.is_empty() {
            self.message_history
                .push(("assistant".to_string(), response.clone()));
        }

        Ok(response)
    }

    /// Send message and collect complete response (convenience method)
    pub async fn chat(&mut self, content: &str) -> Result<String, A2aClientError> {
        let mut rx = self.send_message(content).await?;
        self.collect_response(&mut rx).await
    }
}

/// Parse a command string into command name and arguments
pub fn parse_command(input: &str) -> Option<(&str, Option<&str>)> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let without_slash = &trimmed[1..];
    let mut parts = without_slash.splitn(2, ' ');
    let cmd = parts.next()?;
    let arg = parts.next().map(str::trim);

    Some((cmd, arg))
}

/// Execute a parsed command on a chat session
pub async fn execute_command(
    session: &mut ChatSession,
    cmd: &str,
    arg: Option<&str>,
) -> CommandResult {
    match cmd.to_lowercase().as_str() {
        "new" => session.cmd_new().await,
        "clear" => session.cmd_clear(),
        "context" => session.cmd_context(arg.unwrap_or("")),
        "history" => session.cmd_history(),
        "read" => match arg {
            Some(path) => session.cmd_read(path),
            None => CommandResult::Error("Usage: /read <file>".to_string()),
        },
        "list" | "ls" => session.cmd_list(arg),
        "exit" | "quit" | "q" => session.cmd_exit().await,
        "help" | "?" => session.cmd_help(),
        _ => CommandResult::Error(format!("Unknown command: /{cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_mode_parse() {
        assert_eq!("off".parse::<ContextMode>().unwrap(), ContextMode::Off);
        assert_eq!("on".parse::<ContextMode>().unwrap(), ContextMode::On);
        assert_eq!("auto".parse::<ContextMode>().unwrap(), ContextMode::Auto);
        assert_eq!("OFF".parse::<ContextMode>().unwrap(), ContextMode::Off);
        assert_eq!("true".parse::<ContextMode>().unwrap(), ContextMode::On);
        assert_eq!("false".parse::<ContextMode>().unwrap(), ContextMode::Off);
        assert!("invalid".parse::<ContextMode>().is_err());
    }

    #[test]
    fn test_context_mode_display() {
        assert_eq!(ContextMode::Off.to_string(), "off");
        assert_eq!(ContextMode::On.to_string(), "on");
        assert_eq!(ContextMode::Auto.to_string(), "auto");
    }

    #[test]
    fn test_parse_command() {
        assert_eq!(parse_command("/new"), Some(("new", None)));
        assert_eq!(
            parse_command("/read file.rs"),
            Some(("read", Some("file.rs")))
        );
        assert_eq!(
            parse_command("/context auto"),
            Some(("context", Some("auto")))
        );
        assert_eq!(parse_command("/list"), Some(("list", None)));
        assert_eq!(parse_command("/list src/"), Some(("list", Some("src/"))));
        assert_eq!(parse_command("hello"), None);
        assert_eq!(parse_command(""), None);
    }

    #[test]
    fn test_cmd_help() {
        let client = A2aClient::new();
        let session = ChatSession::from_client(client);
        let result = session.cmd_help();

        match result {
            CommandResult::Output(text) => {
                assert!(text.contains("/new"));
                assert!(text.contains("/clear"));
                assert!(text.contains("/read"));
                assert!(text.contains("/exit"));
            }
            _ => panic!("Expected Output"),
        }
    }

    #[test]
    fn test_cmd_context_get() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);
        let result = session.cmd_context("");

        match result {
            CommandResult::Output(text) => {
                assert!(text.contains("auto")); // default mode
            }
            _ => panic!("Expected Output"),
        }
    }

    #[test]
    fn test_cmd_context_set() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        let result = session.cmd_context("on");
        assert!(matches!(result, CommandResult::Success(_)));
        assert_eq!(session.context_mode(), ContextMode::On);

        let result = session.cmd_context("off");
        assert!(matches!(result, CommandResult::Success(_)));
        assert_eq!(session.context_mode(), ContextMode::Off);
    }

    #[test]
    fn test_cmd_context_invalid() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);
        let result = session.cmd_context("invalid");
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_history_empty() {
        let client = A2aClient::new();
        let session = ChatSession::from_client(client);
        let result = session.cmd_history();

        match result {
            CommandResult::Output(text) => {
                assert!(text.contains("No messages"));
            }
            _ => panic!("Expected Output"),
        }
    }

    #[test]
    fn test_cmd_list_current_dir() {
        let client = A2aClient::new();
        let session = ChatSession::from_client(client);
        let result = session.cmd_list(None);

        match result {
            CommandResult::Output(text) => {
                assert!(text.contains("Contents of ."));
            }
            _ => panic!("Expected Output for current directory"),
        }
    }

    #[test]
    fn test_cmd_list_invalid_dir() {
        let client = A2aClient::new();
        let session = ChatSession::from_client(client);
        let result = session.cmd_list(Some("/nonexistent/path/12345"));

        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_read_nonexistent() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);
        let result = session.cmd_read("/nonexistent/file.txt");

        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn test_pending_context() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        // Read Cargo.toml (should exist in test environment)
        let result = session.cmd_read("Cargo.toml");
        assert!(
            matches!(result, CommandResult::Output(_)),
            "Expected Cargo.toml to be readable"
        );

        assert_eq!(session.pending_context().len(), 1);
        assert_eq!(session.pending_context()[0].path, "Cargo.toml");
    }

    #[test]
    fn test_cmd_clear() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        // Add some pending context
        session.pending_context.push(PendingContext {
            path: "test.rs".to_string(),
            content: "fn main() {}".to_string(),
        });

        let result = session.cmd_clear();
        assert!(matches!(result, CommandResult::Success(_)));
        assert!(session.pending_context().is_empty());
    }

    #[tokio::test]
    async fn test_execute_command() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        let result = execute_command(&mut session, "help", None).await;
        assert!(matches!(result, CommandResult::Output(_)));

        let result = execute_command(&mut session, "context", Some("on")).await;
        assert!(matches!(result, CommandResult::Success(_)));

        let result = execute_command(&mut session, "unknown", None).await;
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[tokio::test]
    async fn test_cmd_exit() {
        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);
        let result = session.cmd_exit().await;
        assert!(matches!(result, CommandResult::Exit));
    }
}
