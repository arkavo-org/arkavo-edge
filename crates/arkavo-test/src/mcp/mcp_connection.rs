use super::server::Tool as McpTool;
use super::{
    device_manager::DeviceManager,
    device_tools::DeviceManagementKit,
    filesystem_tools::FileSystemKit,
    git_tools::{GitBranchKit, GitCommitKit, GitDiffKit, GitLogKit, GitRemoteKit, GitStatusKit},
    github_tools::{
        GitHubIssueCreateKit, GitHubIssueListKit, GitHubPrCreateKit, GitHubPrListKit,
        GitHubPrMergeKit, GitHubReleaseCreateKit, GitHubRepoCloneKit,
    },
    ios_tools::{ScreenCaptureKit, UiInteractionKit, UiQueryKit},
    simulator_tools::SimulatorControl,
    tui_interaction_kit::TuiInteractionKit,
    tui_keyboard_tools::TuiKeyboardKit,
    tui_screenshot_tool::TuiScreenshotKit,
    tui_test_harness::TuiTestHarness,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Handle, Runtime};

#[derive(Debug, Clone)]
pub enum McpConnection {
    InProcess(InProcessMcp),
}

#[derive(Clone)]
pub struct InProcessMcp {
    pub tools: Arc<HashMap<String, Box<dyn McpTool>>>,
    pub runtime_handle: Handle,
    // Keep the owned runtime only if we created it
    owned_runtime: Option<Arc<Runtime>>,
}

impl std::fmt::Debug for InProcessMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessMcp")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .field("has_owned_runtime", &self.owned_runtime.is_some())
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
        // Check if we're already in a Tokio runtime
        let (runtime_handle, owned_runtime) = match Handle::try_current() {
            Ok(handle) => {
                // We're already in a runtime, reuse it
                (handle, None)
            }
            Err(_) => {
                // Not in a runtime, create a new one
                eprintln!("[McpConnection] Creating new Tokio runtime");
                let runtime = Arc::new(Runtime::new()?);
                let handle = runtime.handle().clone();
                (handle, Some(runtime))
            }
        };

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

        // Add TUI testing tools
        let tui_keyboard = TuiKeyboardKit::new();
        tools.insert(tui_keyboard.schema().name.clone(), Box::new(tui_keyboard));

        let tui_screenshot = TuiScreenshotKit::new();
        tools.insert(
            tui_screenshot.schema().name.clone(),
            Box::new(tui_screenshot),
        );

        let tui_interaction = TuiInteractionKit::new();
        tools.insert(
            tui_interaction.schema().name.clone(),
            Box::new(tui_interaction),
        );

        let tui_harness = TuiTestHarness::new();
        tools.insert(tui_harness.schema().name.clone(), Box::new(tui_harness));

        // Add GitHub tools
        let github_pr_create = GitHubPrCreateKit::new();
        tools.insert(
            github_pr_create.schema().name.clone(),
            Box::new(github_pr_create),
        );

        let github_pr_list = GitHubPrListKit::new();
        tools.insert(
            github_pr_list.schema().name.clone(),
            Box::new(github_pr_list),
        );

        let github_pr_merge = GitHubPrMergeKit::new();
        tools.insert(
            github_pr_merge.schema().name.clone(),
            Box::new(github_pr_merge),
        );

        let github_issue_create = GitHubIssueCreateKit::new();
        tools.insert(
            github_issue_create.schema().name.clone(),
            Box::new(github_issue_create),
        );

        let github_issue_list = GitHubIssueListKit::new();
        tools.insert(
            github_issue_list.schema().name.clone(),
            Box::new(github_issue_list),
        );

        let github_repo_clone = GitHubRepoCloneKit::new();
        tools.insert(
            github_repo_clone.schema().name.clone(),
            Box::new(github_repo_clone),
        );

        let github_release_create = GitHubReleaseCreateKit::new();
        tools.insert(
            github_release_create.schema().name.clone(),
            Box::new(github_release_create),
        );

        // Add any additional tools provided
        tools.extend(additional_tools);

        Ok(Self::InProcess(InProcessMcp {
            tools: Arc::new(tools),
            runtime_handle,
            owned_runtime,
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
                    .ok_or_else(|| format!("Tool not found: {name}"))?;

                // Clone the tools map and tool name to use in the thread
                let tools = mcp.tools.clone();
                let tool_name = name.to_string();
                let handle = mcp.runtime_handle.clone();

                // If we're already in a runtime context, spawn in a separate thread
                if Handle::try_current().is_ok() {
                    // We're in an async context, spawn a separate thread to avoid runtime conflicts
                    std::thread::spawn(move || {
                        let tool = tools
                            .get(&tool_name)
                            .ok_or_else(|| format!("Tool not found: {tool_name}"))?;

                        #[allow(clippy::disallowed_methods)]
                        handle.block_on(async move {
                            tool.execute(args)
                                .await
                                .map_err(|e| format!("Tool execution failed: {e}"))
                        })
                    })
                    .join()
                    .map_err(|_| "Tool execution thread panicked".to_string())?
                } else {
                    // We're not in an async context, can block directly
                    #[allow(clippy::disallowed_methods)]
                    handle.block_on(async move {
                        let tool = tools
                            .get(&tool_name)
                            .ok_or_else(|| format!("Tool not found: {tool_name}"))?;

                        tool.execute(args)
                            .await
                            .map_err(|e| format!("Tool execution failed: {e}"))
                    })
                }
            }
        }
    }
}

impl Drop for InProcessMcp {
    fn drop(&mut self) {
        // Only shutdown the runtime if we own it
        if let Some(runtime) = self.owned_runtime.take() {
            eprintln!("[McpConnection] Shutting down owned runtime");
            // If we're in an async context, use shutdown_background to avoid blocking
            if Handle::try_current().is_ok()
                && let Ok(rt) = Arc::try_unwrap(runtime) {
                    rt.shutdown_background();
                }
            // If not in async context, the runtime will shut down normally on drop
        }
    }
}
