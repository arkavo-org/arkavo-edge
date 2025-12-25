use crate::browser::BrowserTool;
use crate::code_review::CodeReviewTool;
use crate::context_control::ContextRestoreTool;
use crate::filesystem::FileSystemKit;
use crate::git::{GitBranchKit, GitCommitKit, GitDiffKit, GitLogKit, GitRemoteKit, GitStatusKit};
use crate::github::{
    GitHubIssueCreateKit, GitHubIssueListKit, GitHubPrCreateKit, GitHubPrListKit, GitHubPrMergeKit,
    GitHubReleaseCreateKit, GitHubRepoCloneKit,
};
use crate::github_checks::GitHubChecksTool;
use crate::github_org_knowledge::{
    GitHubCiStatusTool, GitHubOrgOverviewTool, GitHubOrgReposTool, GitHubRelatedIssuesTool,
};
use crate::github_review::GitHubReviewTool;
use crate::health_check::HealthCheckTool;
use crate::osv::OsvTool;
use crate::semgrep::SemgrepTool;
use crate::server::Tool;
use crate::shell_exec::ShellExecTool;
use crate::syft::SyftTool;
use crate::tdf::{TdfEncryptTool, TdfHelpTool, TdfInfoTool};
#[cfg(feature = "iroh")]
use crate::tdf::{TdfFetchTool, TdfStageTool};
use crate::test_runner::TestRunnerTool;
use crate::time_sync::{GetAgentTimeTool, GetTimeStatusTool, SyncAgentTimeTool};
use crate::web_search::WebSearchTool;
use arkavo_mcp::ToolSchema;
use arkavo_memory::MemoryStorage;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// MCP Tool definition to avoid circular dependency with arkavo-protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Option<Value>,
}

/// Trait for MCP connection abstraction to avoid circular dependency
pub trait McpClient: Send + Sync {
    fn list_tools(&self) -> Result<Vec<McpTool>, Box<dyn std::error::Error>>;
    fn call_tool(
        &self,
        tool_name: &str,
        args: Value,
        llm_origin: &str,
    ) -> Result<Value, Box<dyn std::error::Error>>;
}

/// Wrapper that adapts an MCP tool to the Tool trait
struct McpToolWrapper {
    mcp_client: Arc<dyn McpClient>,
    tool_schema: ToolSchema,
    tool_name: String,
}

#[async_trait]
impl Tool for McpToolWrapper {
    async fn execute(&self, params: Value) -> crate::Result<Value> {
        self.mcp_client
            .call_tool(&self.tool_name, params, "arkavo-router")
            .map_err(|e| crate::ToolError::Mcp(e.to_string()))
    }

    fn schema(&self) -> &ToolSchema {
        &self.tool_schema
    }
}

/// Level of detail to return when discovering tools
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetailLevel {
    /// Return only tool names
    NameOnly,
    /// Return names and descriptions
    NameAndDescription,
    /// Return complete tool schemas with parameters
    FullSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub category: String,
    pub description: String,
    pub schema: serde_json::Value,
}

/// Minimal tool information for progressive disclosure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimalToolInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new registry with all built-in tools registered.
    /// Use `empty()` for small models that need progressive tool discovery.
    ///
    /// # Arguments
    /// * `storage` - Shared memory storage for tools that need persistence
    pub fn new(storage: Arc<MemoryStorage>) -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };

        registry.register_all(storage);
        registry
    }

    /// Create an empty registry without built-in tools.
    /// Use this for small models (3B) where progressive tool discovery is essential.
    /// Only MCP tools from the agent's config will be projected into this registry.
    pub fn empty() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Create a ToolRegistry from an MCP connection
    /// This dynamically discovers tools from the MCP server
    ///
    /// # Arguments
    /// * `mcp_client` - MCP client connection
    /// * `storage` - Shared memory storage for tools that need persistence
    pub fn from_mcp_connection(
        mcp_client: Arc<dyn McpClient>,
        storage: Arc<MemoryStorage>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Start with all native tools (filesystem, browser, git, time, etc.)
        let mut registry = Self::new(storage);

        // Add MCP tools on top (they can override native tools if needed)
        let mcp_tools = mcp_client.list_tools()?;

        for mcp_tool in mcp_tools {
            // Extract parameters from input_schema
            // MCP protocol may return the full schema with name/description/parameters
            // or just the parameters schema directly
            let parameters = if let Some(input_schema) = mcp_tool.input_schema {
                // Check if it has a "parameters" field (full tool definition)
                if let Some(params) = input_schema.get("parameters") {
                    params.clone()
                } else {
                    // Already just the parameters schema
                    input_schema
                }
            } else {
                serde_json::json!({})
            };

            let tool_schema = ToolSchema {
                name: mcp_tool.name.clone(),
                aliases: None,
                description: mcp_tool.description.clone(),
                parameters,
            };

            let wrapper = McpToolWrapper {
                mcp_client: Arc::clone(&mcp_client),
                tool_schema,
                tool_name: mcp_tool.name.clone(),
            };

            registry.register(&mcp_tool.name, Box::new(wrapper));
        }

        Ok(registry)
    }

    /// Create a ToolRegistry from MCP if available, otherwise use default hardcoded tools
    ///
    /// # Arguments
    /// * `mcp_client` - Optional MCP client connection
    /// * `storage` - Shared memory storage for tools that need persistence
    pub fn from_mcp_or_default(
        mcp_client: Option<Arc<dyn McpClient>>,
        storage: Arc<MemoryStorage>,
    ) -> Self {
        if let Some(client) = mcp_client {
            Self::from_mcp_connection(client, storage.clone()).unwrap_or_else(|e| {
                eprintln!(
                    "Warning: Failed to load MCP tools, falling back to defaults: {}",
                    e
                );
                Self::new(storage)
            })
        } else {
            Self::new(storage)
        }
    }

    fn register_all(&mut self, storage: Arc<MemoryStorage>) {
        self.register("filesystem_tools", Box::new(FileSystemKit::new()));
        self.register("browser_cdp", Box::new(BrowserTool::new()));
        self.register("gh_checks", Box::new(GitHubChecksTool::new()));
        self.register("gh_pr_review", Box::new(GitHubReviewTool::new()));

        // Git tools
        self.register("git_status", Box::new(GitStatusKit::new()));
        self.register("git_diff", Box::new(GitDiffKit::new()));
        self.register("git_commit", Box::new(GitCommitKit::new()));
        self.register("git_branch", Box::new(GitBranchKit::new()));
        self.register("git_log", Box::new(GitLogKit::new()));
        self.register("git_remote", Box::new(GitRemoteKit::new()));

        // Only register security tools if binaries are installed
        if Self::is_binary_available("osv-scanner") {
            self.register("deps_osv", Box::new(OsvTool::new()));
        }
        if Self::is_binary_available("semgrep") {
            self.register("sec_semgrep", Box::new(SemgrepTool::new()));
        }
        if Self::is_binary_available("syft") {
            self.register("sbom_syft", Box::new(SyftTool::new()));
        }

        self.register("test_run", Box::new(TestRunnerTool::new()));
        self.register("get_system_health", Box::new(HealthCheckTool::new()));

        let sync_tool = SyncAgentTimeTool::new();
        let sync_state = sync_tool.last_sync_state();

        self.register("get_agent_time", Box::new(GetAgentTimeTool::new()));
        self.register("sync_agent_time", Box::new(sync_tool));
        self.register(
            "get_time_status",
            Box::new(GetTimeStatusTool::new(sync_state)),
        );

        // GitHub Issue Management tools
        self.register("github_issue_create", Box::new(GitHubIssueCreateKit::new()));
        self.register("github_issue_list", Box::new(GitHubIssueListKit::new()));

        // GitHub PR tools
        self.register("github_pr_create", Box::new(GitHubPrCreateKit::new()));
        self.register("github_pr_list", Box::new(GitHubPrListKit::new()));
        self.register("github_pr_merge", Box::new(GitHubPrMergeKit::new()));

        // GitHub Repository tools
        self.register("github_repo_clone", Box::new(GitHubRepoCloneKit::new()));
        self.register(
            "github_release_create",
            Box::new(GitHubReleaseCreateKit::new()),
        );

        // GitHub Org Knowledge tools
        self.register("github_org_repos", Box::new(GitHubOrgReposTool::new()));
        self.register(
            "github_related_issues",
            Box::new(GitHubRelatedIssuesTool::new()),
        );
        self.register("github_ci_status", Box::new(GitHubCiStatusTool::new()));
        self.register(
            "github_org_overview",
            Box::new(GitHubOrgOverviewTool::new()),
        );

        // TDF (Trusted Data Format) tools
        self.register("tdf_encrypt", Box::new(TdfEncryptTool::new()));
        self.register("tdf_info", Box::new(TdfInfoTool::new()));
        self.register("tdf_help", Box::new(TdfHelpTool::new()));

        // TDF Iroh P2P transport tools (feature-gated)
        #[cfg(feature = "iroh")]
        {
            self.register("tdf_stage", Box::new(TdfStageTool::new()));
            self.register("tdf_fetch", Box::new(TdfFetchTool::new()));
        }

        // Shell command execution tool
        self.register("shell_exec", Box::new(ShellExecTool::new()));

        // Web search tool
        self.register("web_search", Box::new(WebSearchTool::new()));

        // Code review tool
        self.register("code_review", Box::new(CodeReviewTool::new()));

        // Context control tool
        self.register(
            "context_restore",
            Box::new(ContextRestoreTool::new(storage)),
        );
    }

    pub fn register(&mut self, name: &str, tool: Box<dyn Tool>) {
        self.tools.insert(name.to_string(), tool);
    }

    /// Check if a binary is available in PATH
    fn is_binary_available(name: &str) -> bool {
        std::process::Command::new(name)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        // Try direct lookup first
        if let Some(tool) = self.tools.get(name) {
            return Some(&**tool);
        }

        // Try alias lookup if direct lookup fails
        self.tools
            .values()
            .find(|tool| {
                let schema = tool.schema();
                schema
                    .aliases
                    .as_ref()
                    .map(|aliases| aliases.iter().any(|alias| alias == name))
                    .unwrap_or(false)
            })
            .map(|boxed| &**boxed)
    }

    pub fn list_tools(&self) -> Vec<ToolInfo> {
        self.tools
            .values()
            .map(|tool| {
                let schema = tool.schema();
                ToolInfo {
                    name: schema.name.clone(),
                    category: Self::categorize_tool(&schema.name),
                    description: schema.description.clone(),
                    schema: serde_json::to_value(&schema.parameters).unwrap_or_default(),
                }
            })
            .collect()
    }

    /// Search for tools matching the query with configurable detail level.
    ///
    /// This method implements progressive tool disclosure, allowing agents to discover
    /// tools on-demand rather than loading all definitions upfront. This significantly
    /// reduces token consumption when working with large tool registries.
    ///
    /// # Arguments
    /// * `query` - Search term to match against tool names and descriptions (case-insensitive)
    /// * `detail` - Level of detail to return (NameOnly, NameAndDescription, FullSchema)
    ///
    /// # Returns
    /// Vector of matching tools with requested detail level
    ///
    /// # Examples
    /// ```ignore
    /// use arkavo_mcp_tools::{ToolRegistry, DetailLevel};
    /// use arkavo_memory::MemoryStorage;
    /// use std::sync::Arc;
    ///
    /// // Requires async setup for MemoryStorage
    /// let storage = Arc::new(MemoryStorage::new().await.unwrap());
    /// let registry = ToolRegistry::new(storage);
    ///
    /// // Get just names for initial discovery
    /// let tools = registry.search_tools("github", DetailLevel::NameOnly);
    ///
    /// // Get names and descriptions for more context
    /// let tools = registry.search_tools("security", DetailLevel::NameAndDescription);
    ///
    /// // Get full schemas when ready to use
    /// let tools = registry.search_tools("semgrep", DetailLevel::FullSchema);
    /// ```
    ///
    /// # Performance
    /// This method is optimized for large tool registries by using lazy loading.
    /// Only the requested detail level is computed, reducing memory usage and
    /// token consumption by up to 98% compared to loading all tool definitions.
    pub fn search_tools(&self, query: &str, detail: DetailLevel) -> Vec<MinimalToolInfo> {
        if query.trim().is_empty() {
            return self
                .tools
                .values()
                .map(|tool| self.build_minimal_info(tool.schema(), detail))
                .collect();
        }

        // Split query by whitespace, then also split each word by underscores/hyphens
        // This ensures "list_agents" matches tool name "list_agents" (exact or tokenized)
        let query_words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .flat_map(|s| {
                // Keep the original word AND its tokenized parts
                let parts: Vec<String> = s.split(&['_', '-'][..]).map(|p| p.to_string()).collect();
                if parts.len() > 1 {
                    // Include both the full word and its parts
                    let mut result = vec![s.to_string()];
                    result.extend(parts);
                    result
                } else {
                    vec![s.to_string()]
                }
            })
            .collect();

        // Collect matching tools with priority scores
        let mut scored_results: Vec<(i32, MinimalToolInfo)> = self
            .tools
            .values()
            .filter_map(|tool| {
                let schema = tool.schema();
                let name_lower = schema.name.to_lowercase();
                let desc_lower = schema.description.to_lowercase();

                // Priority scoring: higher = better match
                let mut priority = 0i32;

                // Exact alias match (highest priority - user intent)
                let exact_alias_match = schema
                    .aliases
                    .as_ref()
                    .map(|aliases| {
                        query_words
                            .iter()
                            .any(|q| aliases.iter().any(|a| a.to_lowercase() == *q))
                    })
                    .unwrap_or(false);
                if exact_alias_match {
                    priority += 100;
                }

                // Exact name match
                let exact_name_match = query_words.iter().any(|q| q == &name_lower);
                if exact_name_match {
                    priority += 50;
                }

                // Token-based matching: check if any query word appears in name or description
                let name_words: Vec<&str> = name_lower.split(&['_', '-', ' '][..]).collect();
                let desc_words: Vec<&str> = desc_lower.split_whitespace().collect();

                let name_token_match = query_words
                    .iter()
                    .any(|q_word| name_words.iter().any(|n| n.contains(q_word.as_str())));
                if name_token_match {
                    priority += 10;
                }

                let desc_token_match = query_words
                    .iter()
                    .any(|q_word| desc_words.iter().any(|d| d.contains(q_word.as_str())));
                if desc_token_match {
                    priority += 1;
                }

                // Partial alias match
                let partial_alias_match = schema
                    .aliases
                    .as_ref()
                    .map(|aliases| {
                        query_words.iter().any(|q_word| {
                            aliases.iter().any(|alias| {
                                let alias_lower = alias.to_lowercase();
                                let alias_words: Vec<&str> =
                                    alias_lower.split(&['_', '-', ' '][..]).collect();
                                alias_words.iter().any(|a| a.contains(q_word.as_str()))
                            })
                        })
                    })
                    .unwrap_or(false);
                if partial_alias_match && !exact_alias_match {
                    priority += 20;
                }

                if priority > 0 {
                    Some((priority, self.build_minimal_info(schema, detail)))
                } else {
                    None
                }
            })
            .collect();

        // Sort by priority (highest first)
        scored_results.sort_by(|a, b| b.0.cmp(&a.0));

        let results: Vec<MinimalToolInfo> =
            scored_results.into_iter().map(|(_, info)| info).collect();

        // Log if search returned no results (learning opportunity for new aliases)
        if results.is_empty() && !query.trim().is_empty() {
            let available_tools: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
            tracing::debug!(
                target: "arkavo_tools::search_miss",
                query = %query,
                query_words = ?query_words,
                available_tools = ?available_tools,
                tool_count = self.tools.len(),
                "Tool search returned no results"
            );
        }

        results
    }

    /// Get a list of tool names and descriptions for semantic search
    pub fn get_tool_descriptions(&self) -> Vec<(String, String)> {
        self.tools
            .values()
            .map(|tool| {
                let schema = tool.schema();
                (schema.name.clone(), schema.description.clone())
            })
            .collect()
    }

    /// Get tools by names (for semantic search results)
    pub fn get_tools_by_names(
        &self,
        names: &[String],
        detail: DetailLevel,
    ) -> Vec<MinimalToolInfo> {
        names
            .iter()
            .filter_map(|name| {
                self.get(name)
                    .map(|t| self.build_minimal_info(t.schema(), detail))
            })
            .collect()
    }

    /// Get detailed information for a specific tool by name.
    ///
    /// This method allows loading full tool schema on-demand after discovering
    /// the tool through search_tools().
    ///
    /// # Arguments
    /// * `tool_name` - Exact name of the tool
    ///
    /// # Returns
    /// Full tool information if found, None otherwise
    pub fn get_tool_info(&self, tool_name: &str) -> Option<ToolInfo> {
        self.tools.get(tool_name).map(|tool| {
            let schema = tool.schema();
            ToolInfo {
                name: schema.name.clone(),
                category: Self::categorize_tool(&schema.name),
                description: schema.description.clone(),
                schema: serde_json::to_value(&schema.parameters).unwrap_or_default(),
            }
        })
    }

    /// Build minimal tool info based on requested detail level
    fn build_minimal_info(&self, schema: &ToolSchema, detail: DetailLevel) -> MinimalToolInfo {
        match detail {
            DetailLevel::NameOnly => MinimalToolInfo {
                name: schema.name.clone(),
                category: None,
                description: None,
                schema: None,
                aliases: None,
            },
            DetailLevel::NameAndDescription => MinimalToolInfo {
                name: schema.name.clone(),
                category: Some(Self::categorize_tool(&schema.name)),
                description: Some(schema.description.clone()),
                schema: None,
                aliases: schema.aliases.clone(),
            },
            DetailLevel::FullSchema => MinimalToolInfo {
                name: schema.name.clone(),
                category: Some(Self::categorize_tool(&schema.name)),
                description: Some(schema.description.clone()),
                schema: Some(serde_json::to_value(&schema.parameters).unwrap_or_default()),
                aliases: schema.aliases.clone(),
            },
        }
    }

    fn categorize_tool(name: &str) -> String {
        match name {
            n if n.starts_with("browser_") => "Browser".to_string(),
            n if n.starts_with("sec_") => "Security".to_string(),
            n if n.starts_with("deps_") => "Security".to_string(),
            n if n.starts_with("sbom_") => "Security".to_string(),
            n if n.starts_with("gh_") => "GitHub".to_string(),
            n if n.starts_with("github_") => "GitHub".to_string(),
            n if n.starts_with("test_") => "Testing".to_string(),
            n if n.starts_with("git") => "Git".to_string(),
            n if n.starts_with("code_") => "Code Analysis".to_string(),
            n if n.starts_with("filesystem") => "File System".to_string(),
            n if n.starts_with("shell_") || n == "bash" || n == "exec" => "Shell".to_string(),
            n if n.starts_with("web_") || n == "search" => "Web".to_string(),
            n if n.contains("time") || n.contains("sync") => "System".to_string(),
            _ => "General".to_string(),
        }
    }

    pub fn list_by_category(&self) -> HashMap<String, Vec<ToolInfo>> {
        let mut categorized: HashMap<String, Vec<ToolInfo>> = HashMap::new();

        for tool_info in self.list_tools() {
            categorized
                .entry(tool_info.category.clone())
                .or_default()
                .push(tool_info);
        }

        categorized
    }

    pub fn export_schemas(&self) -> serde_json::Value {
        let tools: Vec<_> = self.list_tools();
        serde_json::json!({
            "version": "1.0",
            "tool_count": tools.len(),
            "tools": tools,
            "categories": self.list_by_category().keys().collect::<Vec<_>>()
        })
    }
}

// Note: Default cannot be implemented for ToolRegistry since it requires async storage initialization.
// Use ToolRegistry::new(storage) or ToolRegistry::empty() instead.

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    async fn create_test_registry() -> ToolRegistry {
        let storage = Arc::new(MemoryStorage::new_test().await.expect("Storage init"));
        ToolRegistry::new(storage)
    }

    #[tokio::test]
    async fn test_registry_creation() {
        let registry = create_test_registry().await;
        assert!(!registry.tools.is_empty());
    }

    #[tokio::test]
    async fn test_tool_retrieval() {
        let registry = create_test_registry().await;
        // Test retrieval with a tool that's always present
        assert!(registry.get("get_agent_time").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_list_tools() {
        let registry = create_test_registry().await;
        let tools = registry.list_tools();
        assert!(!tools.is_empty());
    }

    #[tokio::test]
    async fn test_categorization() {
        let registry = create_test_registry().await;
        let categories = registry.list_by_category();
        // Security tools are only registered if binaries are available
        // Always check for GitHub tools which are always present
        assert!(categories.contains_key("GitHub"));
        // Verify we have at least some categories
        assert!(!categories.is_empty());
    }

    #[tokio::test]
    async fn test_export_schemas() {
        let registry = create_test_registry().await;
        let schemas = registry.export_schemas();
        assert!(schemas.get("version").is_some());
        assert!(schemas.get("tools").is_some());
    }

    #[tokio::test]
    async fn test_search_tools_name_only() {
        let registry = create_test_registry().await;
        let results = registry.search_tools("github", DetailLevel::NameOnly);

        assert!(!results.is_empty(), "Should find GitHub tools");

        for tool in &results {
            assert!(
                tool.name.to_lowercase().contains("github") || tool.name.starts_with("gh_"),
                "Tool name should contain 'github' or start with 'gh_'"
            );
            assert!(
                tool.category.is_none(),
                "NameOnly should not include category"
            );
            assert!(
                tool.description.is_none(),
                "NameOnly should not include description"
            );
            assert!(tool.schema.is_none(), "NameOnly should not include schema");
        }
    }

    #[tokio::test]
    async fn test_search_tools_name_and_description() {
        let registry = create_test_registry().await;
        // Use "time" which is always available instead of "security" which depends on binaries
        let results = registry.search_tools("time", DetailLevel::NameAndDescription);

        assert!(!results.is_empty(), "Should find time-related tools");

        for tool in &results {
            assert!(tool.category.is_some(), "Should include category");
            assert!(tool.description.is_some(), "Should include description");
            assert!(tool.schema.is_none(), "Should not include full schema");
        }
    }

    #[tokio::test]
    async fn test_search_tools_full_schema() {
        let registry = create_test_registry().await;
        // Use "filesystem" which is always available instead of "semgrep" which depends on binaries
        let results = registry.search_tools("filesystem", DetailLevel::FullSchema);

        assert!(!results.is_empty(), "Should find filesystem tools");

        for tool in &results {
            assert!(tool.category.is_some(), "Should include category");
            assert!(tool.description.is_some(), "Should include description");
            assert!(tool.schema.is_some(), "Should include full schema");
        }
    }

    #[tokio::test]
    async fn test_search_tools_case_insensitive() {
        let registry = create_test_registry().await;

        let lower = registry.search_tools("github", DetailLevel::NameOnly);
        let upper = registry.search_tools("GITHUB", DetailLevel::NameOnly);
        let mixed = registry.search_tools("GiTHuB", DetailLevel::NameOnly);

        assert_eq!(
            lower.len(),
            upper.len(),
            "Search should be case-insensitive"
        );
        assert_eq!(
            lower.len(),
            mixed.len(),
            "Search should be case-insensitive"
        );
    }

    #[tokio::test]
    async fn test_search_tools_no_matches() {
        let registry = create_test_registry().await;
        // Use a truly non-matching query (no common words like "tool")
        let results = registry.search_tools("zzqxvwkj_foobar_blorp", DetailLevel::NameOnly);

        assert!(results.is_empty(), "Should return empty vec for no matches");
    }

    #[tokio::test]
    async fn test_search_tools_exact_name_with_underscore() {
        let registry = create_test_registry().await;
        // Search for exact tool name containing underscore (e.g., "shell_exec")
        let results = registry.search_tools("shell_exec", DetailLevel::NameOnly);

        assert!(
            !results.is_empty(),
            "Should find tool by exact name with underscore"
        );
        assert!(
            results.iter().any(|t| t.name == "shell_exec"),
            "Should include shell_exec in results"
        );

        // Also verify tokenized search still works
        let results_shell = registry.search_tools("shell", DetailLevel::NameOnly);
        assert!(
            results_shell.iter().any(|t| t.name == "shell_exec"),
            "Should find shell_exec when searching for 'shell'"
        );
    }

    #[tokio::test]
    async fn test_search_tools_matches_description() {
        let registry = create_test_registry().await;
        let results = registry.search_tools("check", DetailLevel::NameAndDescription);

        assert!(
            !results.is_empty(),
            "Should find tools matching description"
        );
    }

    #[tokio::test]
    async fn test_get_tool_info() {
        let registry = create_test_registry().await;

        if let Some(first_tool) = registry.list_tools().first() {
            let tool_info = registry.get_tool_info(&first_tool.name);
            assert!(tool_info.is_some(), "Should find existing tool");

            let info = tool_info.unwrap();
            assert_eq!(info.name, first_tool.name);
            assert!(!info.description.is_empty());
        }

        let missing = registry.get_tool_info("nonexistent_tool");
        assert!(missing.is_none(), "Should return None for missing tool");
    }

    #[tokio::test]
    async fn test_progressive_disclosure_token_efficiency() {
        let registry = create_test_registry().await;

        let all_tools = registry.list_tools();
        let all_tools_json = serde_json::to_string(&all_tools).unwrap();
        let all_tools_size = all_tools_json.len();

        let minimal_tools = registry.search_tools("github", DetailLevel::NameOnly);
        let minimal_json = serde_json::to_string(&minimal_tools).unwrap();
        let minimal_size = minimal_json.len();

        assert!(
            minimal_size < all_tools_size / 10,
            "NameOnly should use <10% of tokens compared to full list"
        );
    }

    #[test]
    fn test_detail_level_serialization() {
        let level = DetailLevel::NameAndDescription;
        let json = serde_json::to_string(&level).unwrap();
        let deserialized: DetailLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(level, deserialized);
    }
}
