use async_trait::async_trait;
use std::sync::{Arc, RwLock};

use super::config::{ModelInfo, TaskResult};

/// User interaction trait for task execution feedback
///
/// This trait abstracts UI interactions, allowing the task executor
/// to work with different frontends (CLI, GUI, HTTP server, etc.)
/// and enabling unit testing without a real terminal.
#[async_trait]
pub trait TaskUI: Send + Sync {
    /// Display informational status message
    fn status(&self, message: &str);

    /// Display progress update with optional percentage (0-100)
    fn progress(&self, message: &str, percentage: Option<u8>);

    /// Display error message
    fn error(&self, message: &str);

    /// Display warning message
    fn warn(&self, message: &str);

    /// Display success message
    fn success(&self, message: &str);

    /// Display section header for step transitions
    fn section(&self, title: &str);

    /// Prompt user for confirmation, returns true if approved
    async fn confirm(&self, prompt: &str) -> bool;

    /// Prompt user to select from options, returns index or None if cancelled
    async fn select(&self, prompt: &str, options: &[&str]) -> Option<usize>;

    /// Prompt user for text input, returns None if cancelled
    async fn input(&self, prompt: &str) -> Option<String>;

    /// Display discovered models
    fn show_models(&self, local: &[ModelInfo], cloud: &[ModelInfo]);

    /// Display task execution result
    fn show_result(&self, result: &TaskResult);
}

/// Mock UI implementation for testing
///
/// Records all messages and provides configurable responses
/// for interactive prompts.
#[derive(Debug)]
pub struct MockUI {
    /// Recorded messages: (type, message)
    messages: Arc<RwLock<Vec<(MessageType, String)>>>,
    /// Queue of confirmation responses
    confirmations: Arc<RwLock<Vec<bool>>>,
    /// Queue of selection responses
    selections: Arc<RwLock<Vec<Option<usize>>>>,
    /// Queue of input responses
    inputs: Arc<RwLock<Vec<Option<String>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageType {
    Status,
    Progress,
    Error,
    Warn,
    Success,
    Section,
    Models,
    Result,
}

impl Default for MockUI {
    fn default() -> Self {
        Self::new()
    }
}

impl MockUI {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(Vec::new())),
            confirmations: Arc::new(RwLock::new(Vec::new())),
            selections: Arc::new(RwLock::new(Vec::new())),
            inputs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Configure mock to return specified confirmation values
    pub fn with_confirmations(self, responses: Vec<bool>) -> Self {
        *self.confirmations.write().unwrap() = responses;
        self
    }

    /// Configure mock to return specified selection values
    pub fn with_selections(self, responses: Vec<Option<usize>>) -> Self {
        *self.selections.write().unwrap() = responses;
        self
    }

    /// Configure mock to return specified input values
    pub fn with_inputs(self, responses: Vec<Option<String>>) -> Self {
        *self.inputs.write().unwrap() = responses;
        self
    }

    /// Get all recorded messages
    pub fn messages(&self) -> Vec<(MessageType, String)> {
        self.messages.read().unwrap().clone()
    }

    /// Check if any error was recorded
    pub fn has_error(&self) -> bool {
        self.messages
            .read()
            .unwrap()
            .iter()
            .any(|(t, _)| *t == MessageType::Error)
    }

    /// Get error messages
    pub fn errors(&self) -> Vec<String> {
        self.messages
            .read()
            .unwrap()
            .iter()
            .filter(|(t, _)| *t == MessageType::Error)
            .map(|(_, m)| m.clone())
            .collect()
    }

    fn record(&self, msg_type: MessageType, message: &str) {
        self.messages
            .write()
            .unwrap()
            .push((msg_type, message.to_string()));
    }
}

#[async_trait]
impl TaskUI for MockUI {
    fn status(&self, message: &str) {
        self.record(MessageType::Status, message);
    }

    fn progress(&self, message: &str, percentage: Option<u8>) {
        let msg = match percentage {
            Some(p) => format!("[{p}%] {message}"),
            None => message.to_string(),
        };
        self.record(MessageType::Progress, &msg);
    }

    fn error(&self, message: &str) {
        self.record(MessageType::Error, message);
    }

    fn warn(&self, message: &str) {
        self.record(MessageType::Warn, message);
    }

    fn success(&self, message: &str) {
        self.record(MessageType::Success, message);
    }

    fn section(&self, title: &str) {
        self.record(MessageType::Section, title);
    }

    async fn confirm(&self, prompt: &str) -> bool {
        self.record(MessageType::Status, &format!("CONFIRM: {prompt}"));
        let mut confirmations = self.confirmations.write().unwrap();
        if confirmations.is_empty() {
            true // Default to approved
        } else {
            confirmations.remove(0)
        }
    }

    async fn select(&self, prompt: &str, options: &[&str]) -> Option<usize> {
        self.record(
            MessageType::Status,
            &format!("SELECT: {prompt} [{:?}]", options),
        );
        let mut selections = self.selections.write().unwrap();
        if selections.is_empty() {
            Some(0) // Default to first option
        } else {
            selections.remove(0)
        }
    }

    async fn input(&self, prompt: &str) -> Option<String> {
        self.record(MessageType::Status, &format!("INPUT: {prompt}"));
        let mut inputs = self.inputs.write().unwrap();
        if inputs.is_empty() {
            Some(String::new()) // Default to empty
        } else {
            inputs.remove(0)
        }
    }

    fn show_models(&self, local: &[ModelInfo], cloud: &[ModelInfo]) {
        let msg = format!(
            "local: {} models, cloud: {} models",
            local.len(),
            cloud.len()
        );
        self.record(MessageType::Models, &msg);
    }

    fn show_result(&self, result: &TaskResult) {
        let msg = if result.success {
            format!("SUCCESS: {}", result.message)
        } else {
            format!("FAILURE: {}", result.message)
        };
        self.record(MessageType::Result, &msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_ui_records_messages() {
        let ui = MockUI::new();

        ui.status("Starting task");
        ui.progress("Step 1", Some(33));
        ui.warn("Something might be wrong");
        ui.error("Failed to do X");
        ui.success("Completed");

        let messages = ui.messages();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0], (MessageType::Status, "Starting task".to_string()));
        assert_eq!(messages[1], (MessageType::Progress, "[33%] Step 1".to_string()));
        assert!(ui.has_error());
    }

    #[tokio::test]
    async fn test_mock_ui_confirm_default() {
        let ui = MockUI::new();
        assert!(ui.confirm("Continue?").await);
    }

    #[tokio::test]
    async fn test_mock_ui_confirm_configured() {
        let ui = MockUI::new().with_confirmations(vec![false, true]);
        assert!(!ui.confirm("First?").await);
        assert!(ui.confirm("Second?").await);
        assert!(ui.confirm("Third?").await); // defaults to true
    }

    #[tokio::test]
    async fn test_mock_ui_select_configured() {
        let ui = MockUI::new().with_selections(vec![Some(2), None]);
        assert_eq!(ui.select("Pick", &["a", "b", "c"]).await, Some(2));
        assert_eq!(ui.select("Pick", &["a", "b"]).await, None);
    }
}
