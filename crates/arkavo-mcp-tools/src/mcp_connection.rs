use crate::{
    code_analysis::CodeAnalysisKit,
    filesystem::FileSystemKit,
    git::GitStatusKit,
    github::GitHubPrListKit,
    server::Tool,
    state::QueryStateKit,
    tui::{interaction::TuiInteractionKit, keyboard::TuiKeyboardKit, screenshot::TuiScreenshotKit},
};
#[allow(unused_imports)]
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Handle, Runtime};

/// Cross-platform MCP connection providing platform-independent tools
#[derive(Clone)]
pub struct McpConnection {
    tools: Arc<HashMap<String, Arc<dyn Tool>>>,
    runtime_handle: Handle,
}

impl McpConnection {
    /// Creates a new MCP connection with cross-platform tools
    ///
    /// # Panics
    ///
    /// Panics if unable to create a Tokio runtime when no runtime is already active
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

        // Get a handle to the current runtime or create one
        let runtime_handle = Handle::try_current().unwrap_or_else(|_| {
            Runtime::new()
                .expect("Failed to create Tokio runtime")
                .handle()
                .clone()
        });

        // Register cross-platform tools
        tools.insert("filesystem".to_string(), Arc::new(FileSystemKit::new()));
        tools.insert("git_status".to_string(), Arc::new(GitStatusKit::new()));
        tools.insert(
            "github_pr_list".to_string(),
            Arc::new(GitHubPrListKit::new()),
        );
        tools.insert(
            "code_analysis".to_string(),
            Arc::new(CodeAnalysisKit::new()),
        );
        tools.insert("query_state".to_string(), Arc::new(QueryStateKit::new()));

        // TUI tools (work on all platforms with terminal)
        tools.insert("tui_keyboard".to_string(), Arc::new(TuiKeyboardKit::new()));
        tools.insert(
            "tui_screenshot".to_string(),
            Arc::new(TuiScreenshotKit::new()),
        );
        tools.insert(
            "tui_interaction".to_string(),
            Arc::new(TuiInteractionKit::new()),
        );

        Ok(Self {
            tools: Arc::new(tools),
            runtime_handle,
        })
    }

    pub fn list_tools(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn get_tool_schema(&self, name: &str) -> Option<Value> {
        self.tools.get(name).map(|tool| {
            let schema = tool.schema();
            serde_json::json!({
                "name": schema.name,
                "description": schema.description,
                "parameters": schema.parameters
            })
        })
    }

    #[allow(clippy::disallowed_methods)]
    pub fn call_tool(&self, name: &str, args: Value, _llm_origin: &str) -> Result<Value, String> {
        // Verify tool exists
        let _tool = self
            .tools
            .get(name)
            .ok_or_else(|| format!("Tool not found: {name}"))?;

        // Clone the tools map and tool name to use in the thread
        let tools = self.tools.clone();
        let tool_name = name.to_string();
        let handle = self.runtime_handle.clone();

        // If we're already in a runtime context, spawn in a separate thread
        if Handle::try_current().is_ok() {
            // We're in an async context, spawn a separate thread to avoid runtime conflicts
            std::thread::spawn(move || {
                let tool = tools
                    .get(&tool_name)
                    .ok_or_else(|| format!("Tool not found: {tool_name}"))?;

                handle.block_on(async move { tool.execute(args).await.map_err(|e| e.to_string()) })
            })
            .join()
            .map_err(|_| "Thread panic during tool execution".to_string())?
        } else {
            // No runtime conflict, execute directly
            let tool = self
                .tools
                .get(name)
                .ok_or_else(|| format!("Tool not found: {name}"))?;

            self.runtime_handle
                .block_on(async move { tool.execute(args).await.map_err(|e| e.to_string()) })
        }
    }
}
