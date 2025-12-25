use crate::{
    code_analysis::CodeAnalysisKit,
    filesystem::FileSystemKit,
    git::GitStatusKit,
    github::GitHubPrListKit,
    github_org_knowledge::{
        GitHubCiStatusTool, GitHubOrgOverviewTool, GitHubOrgReposTool, GitHubRelatedIssuesTool,
    },
    registry::{DetailLevel, MinimalToolInfo, ToolRegistry},
    server::Tool,
    state::QueryStateKit,
};
use arkavo_memory::MemoryStorage;
#[allow(unused_imports)]
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Handle, Runtime};

/// Cross-platform MCP connection providing platform-independent tools
///
/// Uses progressive tool disclosure: core tools are listed upfront,
/// additional tools can be discovered via `search_tools()`.
#[derive(Clone)]
pub struct McpConnection {
    /// Core tools exposed in list_tools() for progressive disclosure
    tools: Arc<HashMap<String, Arc<dyn Tool>>>,
    /// Full registry for tool discovery and fallback execution
    registry: Arc<ToolRegistry>,
    runtime_handle: Handle,
}

impl std::fmt::Debug for McpConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConnection")
            .field("tools", &self.tools.keys().cloned().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl McpConnection {
    /// Creates a new MCP connection with cross-platform tools
    ///
    /// Core tools are exposed via `list_tools()` for progressive disclosure.
    /// Additional tools can be discovered via `search_tools()` which delegates
    /// to the full `ToolRegistry`.
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

        // Create storage for tools that need persistence
        let storage = Arc::new(
            runtime_handle
                .block_on(MemoryStorage::new())
                .map_err(|e| format!("Failed to initialize storage: {e}"))?,
        );

        // Create the full registry for discovery and fallback
        let registry = Arc::new(ToolRegistry::new(storage));

        // Register core cross-platform tools (progressive disclosure)
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

        // GitHub org knowledge tools
        tools.insert(
            "github_org_repos".to_string(),
            Arc::new(GitHubOrgReposTool::new()),
        );
        tools.insert(
            "github_related_issues".to_string(),
            Arc::new(GitHubRelatedIssuesTool::new()),
        );
        tools.insert(
            "github_ci_status".to_string(),
            Arc::new(GitHubCiStatusTool::new()),
        );
        tools.insert(
            "github_org_overview".to_string(),
            Arc::new(GitHubOrgOverviewTool::new()),
        );

        // TUI tools moved to discovery-only (available via search_tools/registry)

        Ok(Self {
            tools: Arc::new(tools),
            registry,
            runtime_handle,
        })
    }

    pub fn list_tools(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Search for tools matching the query using the full registry.
    ///
    /// This enables progressive tool discovery - LLM can request tools
    /// that aren't in the core set by searching with keywords.
    pub fn search_tools(&self, query: &str, detail: DetailLevel) -> Vec<MinimalToolInfo> {
        self.registry.search_tools(query, detail)
    }

    pub fn get_tool_schema(&self, name: &str) -> Option<Value> {
        // Try core tools first
        if let Some(tool) = self.tools.get(name) {
            let schema = tool.schema();
            return Some(serde_json::json!({
                "name": schema.name,
                "description": schema.description,
                "parameters": schema.parameters
            }));
        }

        // Fall back to registry for discovered tools
        self.registry.get_tool_info(name).map(|info| {
            serde_json::json!({
                "name": info.name,
                "description": info.description,
                "parameters": info.schema
            })
        })
    }

    #[allow(clippy::disallowed_methods)]
    pub fn call_tool(&self, name: &str, args: Value, _llm_origin: &str) -> Result<Value, String> {
        // Clone values needed for execution
        let tools = self.tools.clone();
        let registry = self.registry.clone();
        let tool_name = name.to_string();
        let handle = self.runtime_handle.clone();

        // If we're already in a runtime context, spawn in a separate thread
        if Handle::try_current().is_ok() {
            // We're in an async context, spawn a separate thread to avoid runtime conflicts
            std::thread::spawn(move || {
                // Try core tools first
                if let Some(tool) = tools.get(&tool_name) {
                    return handle.block_on(async move {
                        tool.execute(args).await.map_err(|e| e.to_string())
                    });
                }

                // Fall back to registry for discovered tools
                if let Some(tool) = registry.get(&tool_name) {
                    return handle.block_on(async move {
                        tool.execute(args).await.map_err(|e| e.to_string())
                    });
                }

                Err(format!("Tool not found: {tool_name}"))
            })
            .join()
            .map_err(|_| "Thread panic during tool execution".to_string())?
        } else {
            // No runtime conflict, execute directly
            // Try core tools first
            if let Some(tool) = self.tools.get(name) {
                return self
                    .runtime_handle
                    .block_on(async move { tool.execute(args).await.map_err(|e| e.to_string()) });
            }

            // Fall back to registry for discovered tools
            if let Some(tool) = self.registry.get(name) {
                return self
                    .runtime_handle
                    .block_on(async move { tool.execute(args).await.map_err(|e| e.to_string()) });
            }

            Err(format!("Tool not found: {name}"))
        }
    }
}
