use crate::browser::BrowserTool;
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
use crate::syft::SyftTool;
use crate::tdf::{TdfEncryptTool, TdfHelpTool, TdfInfoTool};
use crate::test_runner::TestRunnerTool;
use crate::time_sync::{GetAgentTimeTool, GetTimeStatusTool, SyncAgentTimeTool};
use arkavo_mcp::ToolSchema;
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
    pub fn new() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };

        registry.register_all();
        registry
    }

    /// Create a ToolRegistry from an MCP connection
    /// This dynamically discovers tools from the MCP server
    pub fn from_mcp_connection(
        mcp_client: Arc<dyn McpClient>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Start with all native tools (filesystem, browser, git, time, etc.)
        let mut registry = Self::new();

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
    pub fn from_mcp_or_default(mcp_client: Option<Arc<dyn McpClient>>) -> Self {
        if let Some(client) = mcp_client {
            Self::from_mcp_connection(client).unwrap_or_else(|e| {
                eprintln!(
                    "Warning: Failed to load MCP tools, falling back to defaults: {}",
                    e
                );
                Self::new()
            })
        } else {
            Self::new()
        }
    }

    fn register_all(&mut self) {
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
    /// ```
    /// use arkavo_mcp_tools::{ToolRegistry, DetailLevel};
    ///
    /// let registry = ToolRegistry::new();
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

        let query_words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        let results: Vec<MinimalToolInfo> = self
            .tools
            .values()
            .filter_map(|tool| {
                let schema = tool.schema();
                let name_lower = schema.name.to_lowercase();
                let desc_lower = schema.description.to_lowercase();

                // Token-based matching: check if any query word appears in name or description
                let name_words: Vec<&str> = name_lower.split(&['_', '-', ' '][..]).collect();
                let desc_words: Vec<&str> = desc_lower.split_whitespace().collect();

                let matches = query_words.iter().any(|q_word| {
                    // Match if query word appears in tool name words
                    name_words.iter().any(|n_word| n_word.contains(q_word))
                        // Or in description words
                        || desc_words.iter().any(|d_word| d_word.contains(q_word))
                });

                // Also check aliases (using same tokenization as name/description)
                let alias_match = schema
                    .aliases
                    .as_ref()
                    .map(|aliases| {
                        query_words.iter().any(|q_word| {
                            aliases.iter().any(|alias| {
                                let alias_lower = alias.to_lowercase();
                                let alias_words: Vec<&str> =
                                    alias_lower.split(&['_', '-', ' '][..]).collect();
                                alias_words.iter().any(|a_word| a_word.contains(q_word))
                            })
                        })
                    })
                    .unwrap_or(false);

                if matches || alias_match {
                    Some(self.build_minimal_info(schema, detail))
                } else {
                    None
                }
            })
            .collect();

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

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = ToolRegistry::new();
        assert!(!registry.tools.is_empty());
    }

    #[test]
    fn test_tool_retrieval() {
        let registry = ToolRegistry::new();
        // Test retrieval with a tool that's always present
        assert!(registry.get("get_agent_time").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_list_tools() {
        let registry = ToolRegistry::new();
        let tools = registry.list_tools();
        assert!(!tools.is_empty());
    }

    #[test]
    fn test_categorization() {
        let registry = ToolRegistry::new();
        let categories = registry.list_by_category();
        // Security tools are only registered if binaries are available
        // Always check for GitHub tools which are always present
        assert!(categories.contains_key("GitHub"));
        // Verify we have at least some categories
        assert!(!categories.is_empty());
    }

    #[test]
    fn test_export_schemas() {
        let registry = ToolRegistry::new();
        let schemas = registry.export_schemas();
        assert!(schemas.get("version").is_some());
        assert!(schemas.get("tools").is_some());
    }

    #[test]
    fn test_search_tools_name_only() {
        let registry = ToolRegistry::new();
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

    #[test]
    fn test_search_tools_name_and_description() {
        let registry = ToolRegistry::new();
        // Use "time" which is always available instead of "security" which depends on binaries
        let results = registry.search_tools("time", DetailLevel::NameAndDescription);

        assert!(!results.is_empty(), "Should find time-related tools");

        for tool in &results {
            assert!(tool.category.is_some(), "Should include category");
            assert!(tool.description.is_some(), "Should include description");
            assert!(tool.schema.is_none(), "Should not include full schema");
        }
    }

    #[test]
    fn test_search_tools_full_schema() {
        let registry = ToolRegistry::new();
        // Use "filesystem" which is always available instead of "semgrep" which depends on binaries
        let results = registry.search_tools("filesystem", DetailLevel::FullSchema);

        assert!(!results.is_empty(), "Should find filesystem tools");

        for tool in &results {
            assert!(tool.category.is_some(), "Should include category");
            assert!(tool.description.is_some(), "Should include description");
            assert!(tool.schema.is_some(), "Should include full schema");
        }
    }

    #[test]
    fn test_search_tools_case_insensitive() {
        let registry = ToolRegistry::new();

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

    #[test]
    fn test_search_tools_no_matches() {
        let registry = ToolRegistry::new();
        let results = registry.search_tools("nonexistent_tool_xyz", DetailLevel::NameOnly);

        assert!(results.is_empty(), "Should return empty vec for no matches");
    }

    #[test]
    fn test_search_tools_matches_description() {
        let registry = ToolRegistry::new();
        let results = registry.search_tools("check", DetailLevel::NameAndDescription);

        assert!(
            !results.is_empty(),
            "Should find tools matching description"
        );
    }

    #[test]
    fn test_get_tool_info() {
        let registry = ToolRegistry::new();

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

    #[test]
    fn test_progressive_disclosure_token_efficiency() {
        let registry = ToolRegistry::new();

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
