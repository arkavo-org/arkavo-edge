mod session;
mod ui;

pub use session::ChatSession;
pub use ui::display_response;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChatError {
    #[error("Failed to initialize chat session: {0}")]
    InitializationError(String),

    #[error("Failed to process input: {0}")]
    InputProcessingError(String),

    #[error("Failed to generate response: {0}")]
    ResponseGenerationError(String),
}

/// Configuration for chat sessions
#[derive(Debug, Clone)]
pub struct ChatConfig {
    /// Whether to use interactive mode
    pub interactive: bool,
    
    /// Whether to display thinking animation
    pub show_thinking: bool,
    
    /// Maximum history messages to keep
    pub max_history: usize,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            interactive: true,
            show_thinking: true,
            max_history: 10,
        }
    }
}

/// Chat message with role and content
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Role (user or assistant)
    pub role: ChatRole,
    
    /// Message content
    pub content: String,
}

/// Role in a chat conversation
#[derive(Debug, Clone, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

impl ChatMessage {
    /// Create a new user message
    pub fn user(content: &str) -> Self {
        Self {
            role: ChatRole::User,
            content: content.to_string(),
        }
    }
    
    /// Create a new assistant message
    pub fn assistant(content: &str) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.to_string(),
        }
    }
    
    /// Create a new system message
    pub fn system(content: &str) -> Self {
        Self {
            role: ChatRole::System,
            content: content.to_string(),
        }
    }
}

/// Format a single message for the model
pub fn format_message(message: &ChatMessage) -> String {
    let role = match message.role {
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::System => "system",
    };
    
    format!("<|im_start|>{}\n{}<|im_end|>\n", role, message.content)
}

/// Format a list of messages for the model
pub fn format_messages(messages: &[ChatMessage]) -> String {
    messages.iter().map(format_message).collect()
}