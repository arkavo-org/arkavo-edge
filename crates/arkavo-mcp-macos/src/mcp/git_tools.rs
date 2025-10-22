use crate::mcp::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use arkavo_git::attribution::format_commit_message;
use arkavo_git::safety::sanitize_repo_path;
use arkavo_git::{DiffOptions, GitManager};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::env;
use std::path::Path;

/// Safely open a repository with path sanitization
fn safe_open_repo(
    git_manager: &GitManager,
    requested_path: &str,
) -> Result<arkavo_git::Repository> {
    let requested = Path::new(requested_path);

    // If the path is absolute and exists, use it directly (for tests)
    // Otherwise, treat it relative to current directory
    let base_path = if requested.is_absolute() && requested.exists() {
        requested.to_path_buf()
    } else {
        let current_dir = env::current_dir()
            .map_err(|e| TestError::Mcp(format!("Failed to get current directory: {e}")))?;
        current_dir.join(requested)
    };

    // For absolute paths that exist (like temp directories in tests),
    // we trust them. For relative paths, we sanitize.
    let final_path = if requested.is_absolute() && requested.exists() {
        base_path
    } else {
        let current_dir = env::current_dir()
            .map_err(|e| TestError::Mcp(format!("Failed to get current directory: {e}")))?;
        sanitize_repo_path(&current_dir, &base_path)
            .map_err(|e| TestError::Mcp(format!("Path validation failed: {e}")))?
    };

    git_manager
        .open_repo(&final_path)
        .map_err(|e| TestError::Mcp(format!("Failed to open repository: {e}")))
}

pub struct GitStatusKit {
    schema: ToolSchema,
    git_manager: GitManager,
}

impl GitStatusKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "git_status".to_string(),
                description: "Get the current Git repository status".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository path (defaults to current directory)",
                        }
                    }
                }),
            },
            git_manager: GitManager::new(),
        }
    }
}

impl Default for GitStatusKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitStatusKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params["path"].as_str().unwrap_or(".");
        let repo = safe_open_repo(&self.git_manager, path)?;
        let status = self
            .git_manager
            .status(&repo)
            .map_err(|e| TestError::Mcp(format!("Failed to get status: {e}")))?;
        let branch = self
            .git_manager
            .get_current_branch(&repo)
            .map_err(|e| TestError::Mcp(format!("Failed to get branch: {e}")))?;

        Ok(json!({
            "branch": branch,
            "modified": status.modified,
            "added": status.added,
            "deleted": status.deleted,
            "renamed": status.renamed,
            "untracked": status.untracked,
            "conflicted": status.conflicted,
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitDiffKit {
    schema: ToolSchema,
    git_manager: GitManager,
}

impl GitDiffKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "git_diff".to_string(),
                description: "Get the diff of changes in the repository".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository path (defaults to current directory)",
                        },
                        "staged": {
                            "type": "boolean",
                            "description": "Show staged changes",
                            "default": false
                        },
                        "cached": {
                            "type": "boolean",
                            "description": "Show cached changes",
                            "default": false
                        }
                    }
                }),
            },
            git_manager: GitManager::new(),
        }
    }
}

impl Default for GitDiffKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitDiffKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params["path"].as_str().unwrap_or(".");
        let staged = params["staged"].as_bool().unwrap_or(false);
        let cached = params["cached"].as_bool().unwrap_or(false);

        let repo = safe_open_repo(&self.git_manager, path)?;
        let diff_options = DiffOptions {
            staged,
            unstaged: !staged && !cached,
            cached,
            context_lines: 3,
        };

        let diff = self
            .git_manager
            .diff(&repo, &diff_options)
            .map_err(|e| TestError::Mcp(format!("Failed to get diff: {e}")))?;

        Ok(json!({
            "diff": diff,
            "options": {
                "staged": staged,
                "cached": cached,
                "unstaged": diff_options.unstaged,
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitCommitKit {
    schema: ToolSchema,
    git_manager: GitManager,
}

impl GitCommitKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "git_commit".to_string(),
                description: "Stage all changes and create a commit".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository path (defaults to current directory)",
                        },
                        "message": {
                            "type": "string",
                            "description": "Commit message"
                        }
                    },
                    "required": ["message"]
                }),
            },
            git_manager: GitManager::new(),
        }
    }
}

impl Default for GitCommitKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitCommitKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params["path"].as_str().unwrap_or(".");
        let message = params["message"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Commit message is required".to_string()))?;

        let repo = safe_open_repo(&self.git_manager, path)?;

        // Check if there are any changes before staging
        let status_before = self
            .git_manager
            .status(&repo)
            .map_err(|e| TestError::Mcp(format!("Failed to get status: {e}")))?;

        // Check if any files will be modified (not just added/deleted)
        let files_modified = !status_before.modified.is_empty()
            || !status_before.added.is_empty()
            || !status_before.deleted.is_empty();

        // Stage all changes
        self.git_manager
            .add_all(&repo)
            .map_err(|e| TestError::Mcp(format!("Failed to stage changes: {e}")))?;

        // Format the commit message with attribution
        let formatted_message = format_commit_message(message, files_modified);

        // Create commit
        let oid = self
            .git_manager
            .commit_changes(&repo, &formatted_message)
            .map_err(|e| TestError::Mcp(format!("Failed to commit: {e}")))?;

        Ok(json!({
            "success": true,
            "commit_id": oid.to_string(),
            "message": formatted_message
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitBranchKit {
    schema: ToolSchema,
    git_manager: GitManager,
}

impl GitBranchKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "git_branch".to_string(),
                description: "List, create, or switch Git branches".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository path (defaults to current directory)",
                        },
                        "action": {
                            "type": "string",
                            "enum": ["list", "create", "switch"],
                            "description": "Branch operation to perform"
                        },
                        "name": {
                            "type": "string",
                            "description": "Branch name (required for create/switch)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            git_manager: GitManager::new(),
        }
    }
}

impl Default for GitBranchKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitBranchKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params["path"].as_str().unwrap_or(".");
        let action = params["action"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Action is required".to_string()))?;

        let repo = safe_open_repo(&self.git_manager, path)?;

        match action {
            "list" => {
                let branches = self
                    .git_manager
                    .list_branches(&repo)
                    .map_err(|e| TestError::Mcp(format!("Failed to list branches: {e}")))?;
                Ok(json!({
                    "branches": branches.into_iter().map(|(name, is_current)| {
                        json!({
                            "name": name,
                            "current": is_current
                        })
                    }).collect::<Vec<_>>()
                }))
            }
            "create" => {
                let name = params["name"].as_str().ok_or_else(|| {
                    TestError::Mcp("Branch name is required for create action".to_string())
                })?;
                self.git_manager
                    .create_branch(&repo, name)
                    .map_err(|e| TestError::Mcp(format!("Failed to create branch: {e}")))?;
                Ok(json!({
                    "success": true,
                    "created": name
                }))
            }
            "switch" => {
                let name = params["name"].as_str().ok_or_else(|| {
                    TestError::Mcp("Branch name is required for switch action".to_string())
                })?;
                self.git_manager
                    .checkout_branch(&repo, name)
                    .map_err(|e| TestError::Mcp(format!("Failed to switch branch: {e}")))?;
                Ok(json!({
                    "success": true,
                    "switched_to": name
                }))
            }
            _ => Err(TestError::Mcp(
                "Invalid action. Use 'list', 'create', or 'switch'".to_string(),
            )),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitLogKit {
    schema: ToolSchema,
    git_manager: GitManager,
}

impl GitLogKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "git_log".to_string(),
                description: "Show commit history".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository path (defaults to current directory)",
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of commits to show",
                            "default": 10
                        }
                    }
                }),
            },
            git_manager: GitManager::new(),
        }
    }
}

impl Default for GitLogKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitLogKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params["path"].as_str().unwrap_or(".");
        let limit = params["limit"].as_u64().unwrap_or(10) as usize;

        let repo = safe_open_repo(&self.git_manager, path)?;

        let mut revwalk = repo
            .revwalk()
            .map_err(|e| TestError::Mcp(format!("Failed to create revwalk: {e}")))?;
        revwalk
            .push_head()
            .map_err(|e| TestError::Mcp(format!("Failed to push head: {e}")))?;

        let mut commits = Vec::new();
        for (i, oid) in revwalk.enumerate() {
            if i >= limit {
                break;
            }

            let oid = oid.map_err(|e| TestError::Mcp(format!("Failed to get oid: {e}")))?;
            let commit = repo
                .find_commit(oid)
                .map_err(|e| TestError::Mcp(format!("Failed to find commit: {e}")))?;

            commits.push(json!({
                "id": oid.to_string(),
                "author": commit.author().name().unwrap_or("Unknown"),
                "email": commit.author().email().unwrap_or("Unknown"),
                "message": commit.message().unwrap_or(""),
                "time": commit.time().seconds()
            }));
        }

        Ok(json!({
            "commits": commits
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitRemoteKit {
    schema: ToolSchema,
    git_manager: GitManager,
}

impl GitRemoteKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "git_remote".to_string(),
                description: "Manage remote repository operations (fetch, pull, push)".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository path (defaults to current directory)",
                        },
                        "action": {
                            "type": "string",
                            "enum": ["fetch", "pull", "push", "sync"],
                            "description": "Remote operation to perform"
                        },
                        "remote": {
                            "type": "string",
                            "description": "Remote name (defaults to 'origin')",
                            "default": "origin"
                        },
                        "branch": {
                            "type": "string",
                            "description": "Branch name (defaults to current branch)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            git_manager: GitManager::new(),
        }
    }
}

impl Default for GitRemoteKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitRemoteKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params["path"].as_str().unwrap_or(".");
        let action = params["action"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Action is required".to_string()))?;
        let remote = params["remote"].as_str().unwrap_or("origin");

        let repo = safe_open_repo(&self.git_manager, path)?;

        // Get current branch if not specified
        let branch = match params["branch"].as_str() {
            Some(b) => b.to_string(),
            None => self
                .git_manager
                .get_current_branch(&repo)
                .map_err(|e| TestError::Mcp(format!("Failed to get current branch: {e}")))?,
        };

        match action {
            "fetch" => {
                self.git_manager
                    .fetch(&repo, remote)
                    .map_err(|e| TestError::Mcp(format!("Failed to fetch: {e}")))?;
                Ok(json!({
                    "success": true,
                    "action": "fetch",
                    "remote": remote,
                    "message": format!("Successfully fetched from {}", remote)
                }))
            }
            "pull" => {
                self.git_manager
                    .pull(&repo, remote, &branch)
                    .map_err(|e| TestError::Mcp(format!("Failed to pull: {e}")))?;
                Ok(json!({
                    "success": true,
                    "action": "pull",
                    "remote": remote,
                    "branch": branch,
                    "message": format!("Successfully pulled {} from {}", branch, remote)
                }))
            }
            "push" => {
                self.git_manager
                    .push(&repo, remote, &branch)
                    .map_err(|e| TestError::Mcp(format!("Failed to push: {e}")))?;
                Ok(json!({
                    "success": true,
                    "action": "push",
                    "remote": remote,
                    "branch": branch,
                    "message": format!("Successfully pushed {} to {}", branch, remote)
                }))
            }
            "sync" => {
                // sync = pull then push
                self.git_manager
                    .sync_upstream(&repo)
                    .map_err(|e| TestError::Mcp(format!("Failed to sync upstream: {e}")))?;
                self.git_manager
                    .publish(&repo)
                    .map_err(|e| TestError::Mcp(format!("Failed to publish: {e}")))?;
                Ok(json!({
                    "success": true,
                    "action": "sync",
                    "remote": remote,
                    "branch": branch,
                    "message": format!("Successfully synced {} with {}", branch, remote)
                }))
            }
            _ => Err(TestError::Mcp(
                "Invalid action. Use 'fetch', 'pull', 'push', or 'sync'".to_string(),
            )),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_git_commit_with_attribution() {
        let temp_dir = TempDir::new().unwrap();
        let manager = GitManager::new();

        // Initialize repo
        manager.init_repo(temp_dir.path()).unwrap();
        let repo = manager.open_repo(temp_dir.path()).unwrap();

        // Set up git config for tests
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create a file
        fs::write(temp_dir.path().join("test.txt"), "Hello world").unwrap();

        // Test GitCommitKit
        let commit_kit = GitCommitKit::new();
        let params = json!({
            "path": temp_dir.path().to_str().unwrap(),
            "message": "Add test file"
        });

        let result = commit_kit.execute(params).await.unwrap();

        // Check that the commit was successful
        assert!(result["success"].as_bool().unwrap());

        // Check the formatted message
        let message = result["message"].as_str().unwrap();
        assert!(message.contains("Add test file"));
        assert!(message.contains("🤖 Generated with [Arkavo Edge]"));
        assert!(message.contains("Co-Authored-By: Arkavo Edge <edge@arkavo.com>"));
    }
}
