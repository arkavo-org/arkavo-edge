use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[async_trait::async_trait]
pub trait UserInterface: Send + Sync {
    async fn initialize(&mut self) -> Result<()>;

    async fn display(&mut self, content: UIContent) -> Result<()>;

    async fn next_event(&mut self) -> Result<UIEvent>;

    fn is_running(&self) -> bool;

    async fn shutdown(self: Box<Self>) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UIContent {
    Text(String),

    Markdown(String),

    Html {
        html: String,
        css: String,
        js: String,
    },

    StreamChunk {
        content: String,
        is_complete: bool,
    },

    Error(String),

    StatusUpdate(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UIEvent {
    TextInput(String),

    FormSubmit { data: HashMap<String, String> },

    ButtonClick { id: String, label: String },

    Shutdown,
}
