use crate::mcp::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use arkavo_git::{DiffOptions, GitManager};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;

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
        let repo = self
            .git_manager
            .open_repo(Path::new(path))
            .map_err(|e| TestError::Mcp(format!("Failed to open repository: {}", e)))?;
        let status = self
            .git_manager
            .status(&repo)
            .map_err(|e| TestError::Mcp(format!("Failed to get status: {}", e)))?;
        let branch = self
            .git_manager
            .get_current_branch(&repo)
            .map_err(|e| TestError::Mcp(format!("Failed to get branch: {}", e)))?;

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

        let repo = self
            .git_manager
            .open_repo(Path::new(path))
            .map_err(|e| TestError::Mcp(format!("Failed to open repository: {}", e)))?;
        let diff_options = DiffOptions {
            staged,
            unstaged: !staged && !cached,
            cached,
            context_lines: 3,
        };

        let diff = self
            .git_manager
            .diff(&repo, &diff_options)
            .map_err(|e| TestError::Mcp(format!("Failed to get diff: {}", e)))?;

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

        let repo = self
            .git_manager
            .open_repo(Path::new(path))
            .map_err(|e| TestError::Mcp(format!("Failed to open repository: {}", e)))?;

        // Stage all changes
        self.git_manager
            .add_all(&repo)
            .map_err(|e| TestError::Mcp(format!("Failed to stage changes: {}", e)))?;

        // Create commit
        let oid = self
            .git_manager
            .commit_changes(&repo, message)
            .map_err(|e| TestError::Mcp(format!("Failed to commit: {}", e)))?;

        Ok(json!({
            "success": true,
            "commit_id": oid.to_string(),
            "message": message
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

        let repo = self
            .git_manager
            .open_repo(Path::new(path))
            .map_err(|e| TestError::Mcp(format!("Failed to open repository: {}", e)))?;

        match action {
            "list" => {
                let branches = self
                    .git_manager
                    .list_branches(&repo)
                    .map_err(|e| TestError::Mcp(format!("Failed to list branches: {}", e)))?;
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
                    .map_err(|e| TestError::Mcp(format!("Failed to create branch: {}", e)))?;
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
                    .map_err(|e| TestError::Mcp(format!("Failed to switch branch: {}", e)))?;
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

        let repo = self
            .git_manager
            .open_repo(Path::new(path))
            .map_err(|e| TestError::Mcp(format!("Failed to open repository: {}", e)))?;

        let mut revwalk = repo
            .revwalk()
            .map_err(|e| TestError::Mcp(format!("Failed to create revwalk: {}", e)))?;
        revwalk
            .push_head()
            .map_err(|e| TestError::Mcp(format!("Failed to push head: {}", e)))?;

        let mut commits = Vec::new();
        for (i, oid) in revwalk.enumerate() {
            if i >= limit {
                break;
            }

            let oid = oid.map_err(|e| TestError::Mcp(format!("Failed to get oid: {}", e)))?;
            let commit = repo
                .find_commit(oid)
                .map_err(|e| TestError::Mcp(format!("Failed to find commit: {}", e)))?;

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
