use super::server::Tool as McpTool;
use super::{
    device_manager::DeviceManager,
    device_tools::DeviceManagementKit,
    filesystem_tools::FileSystemKit,
    git_tools::{GitBranchKit, GitCommitKit, GitDiffKit, GitLogKit, GitRemoteKit, GitStatusKit},
    ios_tools::{ScreenCaptureKit, UiInteractionKit, UiQueryKit},
    simulator_tools::SimulatorControl,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[derive(Debug, Clone)]
pub enum McpConnection {
    InProcess(InProcessMcp),
}

#[derive(Clone)]
pub struct InProcessMcp {
    pub tools: Arc<HashMap<String, Box<dyn McpTool>>>,
    pub runtime: Option<Arc<Runtime>>,
}

impl std::fmt::Debug for InProcessMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessMcp")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .field("runtime", &self.runtime.is_some())
            .finish()
    }
}

impl McpConnection {
    pub fn new_in_process() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_in_process_with_additional_tools(HashMap::new())
    }

    pub fn new_in_process_with_additional_tools(
        additional_tools: HashMap<String, Box<dyn McpTool>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Always create a dedicated runtime for MCP operations
        // This avoids the "Cannot start a runtime from within a runtime" panic
        let runtime = Some(Arc::new(Runtime::new()?));

        // Create tools with shared device manager
        let device_manager = Arc::new(DeviceManager::new());

        let mut tools: HashMap<String, Box<dyn McpTool>> = HashMap::new();

        // Register all tools
        let simulator_control = SimulatorControl::new();
        tools.insert(
            simulator_control.schema().name.clone(),
            Box::new(simulator_control),
        );

        let device_mgmt = DeviceManagementKit::new(device_manager.clone());
        tools.insert(device_mgmt.schema().name.clone(), Box::new(device_mgmt));

        let screen_capture = ScreenCaptureKit::new(device_manager.clone());
        tools.insert(
            screen_capture.schema().name.clone(),
            Box::new(screen_capture),
        );

        let ui_interaction = UiInteractionKit::new(device_manager.clone());
        tools.insert(
            ui_interaction.schema().name.clone(),
            Box::new(ui_interaction),
        );

        let ui_query = UiQueryKit::new(device_manager);
        tools.insert(ui_query.schema().name.clone(), Box::new(ui_query));

        // Add Git tools
        let git_status = GitStatusKit::new();
        tools.insert(git_status.schema().name.clone(), Box::new(git_status));

        let git_diff = GitDiffKit::new();
        tools.insert(git_diff.schema().name.clone(), Box::new(git_diff));

        let git_commit = GitCommitKit::new();
        tools.insert(git_commit.schema().name.clone(), Box::new(git_commit));

        let git_branch = GitBranchKit::new();
        tools.insert(git_branch.schema().name.clone(), Box::new(git_branch));

        let git_log = GitLogKit::new();
        tools.insert(git_log.schema().name.clone(), Box::new(git_log));

        let git_remote = GitRemoteKit::new();
        tools.insert(git_remote.schema().name.clone(), Box::new(git_remote));

        // Add file system tools
        let filesystem = FileSystemKit::new();
        tools.insert(filesystem.schema().name.clone(), Box::new(filesystem));

        // Add any additional tools provided
        tools.extend(additional_tools);

        Ok(Self::InProcess(InProcessMcp {
            tools: Arc::new(tools),
            runtime,
        }))
    }

    pub fn list_tools(&self) -> Vec<String> {
        match self {
            McpConnection::InProcess(mcp) => mcp.tools.keys().cloned().collect(),
        }
    }

    pub fn get_tool_schema(&self, name: &str) -> Option<Value> {
        match self {
            McpConnection::InProcess(mcp) => mcp
                .tools
                .get(name)
                .map(|tool| serde_json::to_value(tool.schema()).unwrap()),
        }
    }

    pub fn call_tool(&self, name: &str, args: Value, _provider: &str) -> Result<Value, String> {
        match self {
            McpConnection::InProcess(mcp) => {
                // Verify tool exists
                let _tool = mcp
                    .tools
                    .get(name)
                    .ok_or_else(|| format!("Tool not found: {}", name))?;

                // Create a new thread to avoid runtime conflicts
                let args_clone = args.clone();
                let tools = mcp.tools.clone();
                let tool_name = name.to_string();

                std::thread::spawn(move || {
                    // Get the tool again in the new thread
                    let tool = tools
                        .get(&tool_name)
                        .ok_or_else(|| format!("Tool not found: {}", tool_name))?;

                    // Create a new runtime for this thread
                    let thread_rt =
                        Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

                    thread_rt.block_on(async move {
                        tool.execute(args_clone)
                            .await
                            .map_err(|e| format!("Tool execution failed: {}", e))
                    })
                })
                .join()
                .map_err(|_| "Tool execution thread panicked".to_string())?
            }
        }
    }
}
