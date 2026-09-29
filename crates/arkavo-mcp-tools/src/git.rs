use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use arkavo_git::attribution::format_commit_message;
use arkavo_git::{DiffOptions, GitManager};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Open the repository at `requested`, which must resolve inside `root`
/// (MCP-011). An absolute path used to be trusted whenever it existed, which
/// let a call open, read and commit to any repository on the host.
///
/// `open_repo` discovers upward, so a root with no `.git` of its own would
/// open an enclosing repository (a monorepo parent, a dotfiles repo) and let
/// add, commit and diff act outside the workspace. After opening, exactly this
/// is checked: the working directory (or git directory, when bare), git
/// directory and common directory all lie inside `root`; `objects/info/alternates`
/// holds no entry; and `objects`, `objects/info`, `refs`, `HEAD`, `index`,
/// `packed-refs` and `logs` under the git and common directories are not
/// symlinks. A linked worktree therefore passes only when its main repository is
/// inside the root too. The filesystem and TDF tools (`tdf_encrypt`,
/// `tdf_fetch`) refuse to write any `.git` entry, so the workspace cannot forge
/// these through this crate; other crafted-`.git`
/// vectors remain a residual owned by OS confinement.
fn safe_open_repo(
    git_manager: &GitManager,
    root: &Path,
    requested: &str,
) -> Result<arkavo_git::Repository> {
    let path = crate::confine::within_root(root, requested)?;
    let repo = git_manager
        .open_repo(&path)
        .map_err(|e| ToolError::Mcp(format!("Failed to open repository: {e}")))?;
    crate::confine::require_repo_inside_root(root, &repo)?;
    Ok(repo)
}

pub struct GitStatusKit {
    schema: ToolSchema,
    git_manager: GitManager,
    root: PathBuf,
}

impl GitStatusKit {
    pub fn new() -> Self {
        Self::with_root(arkavo_validation::current_workspace_root())
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            schema: ToolSchema {
                name: "git_status".to_string(),
                aliases: Some(vec!["status".to_string()]),
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
        let repo = safe_open_repo(&self.git_manager, &self.root, path)?;
        let status = self
            .git_manager
            .status(&repo)
            .map_err(|e| ToolError::Mcp(format!("Failed to get status: {e}")))?;
        let branch = self
            .git_manager
            .get_current_branch(&repo)
            .map_err(|e| ToolError::Mcp(format!("Failed to get branch: {e}")))?;

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
    root: PathBuf,
}

impl GitDiffKit {
    pub fn new() -> Self {
        Self::with_root(arkavo_validation::current_workspace_root())
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            schema: ToolSchema {
                name: "git_diff".to_string(),
                aliases: Some(vec!["diff".to_string()]),
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

        let repo = safe_open_repo(&self.git_manager, &self.root, path)?;
        let diff_options = DiffOptions {
            staged,
            unstaged: !staged && !cached,
            cached,
            context_lines: 3,
        };

        let diff = self
            .git_manager
            .diff(&repo, &diff_options)
            .map_err(|e| ToolError::Mcp(format!("Failed to get diff: {e}")))?;

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
    root: PathBuf,
}

impl GitCommitKit {
    pub fn new() -> Self {
        Self::with_root(arkavo_validation::current_workspace_root())
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            schema: ToolSchema {
                name: "git_commit".to_string(),
                aliases: Some(vec!["commit".to_string()]),
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
            .ok_or_else(|| ToolError::Mcp("Commit message is required".to_string()))?;

        let repo = safe_open_repo(&self.git_manager, &self.root, path)?;

        // Check if there are any changes before staging
        let status_before = self
            .git_manager
            .status(&repo)
            .map_err(|e| ToolError::Mcp(format!("Failed to get status: {e}")))?;

        // Check if any files will be modified (not just added/deleted)
        let files_modified = !status_before.modified.is_empty()
            || !status_before.added.is_empty()
            || !status_before.deleted.is_empty();

        // Stage all changes
        self.git_manager
            .add_all(&repo)
            .map_err(|e| ToolError::Mcp(format!("Failed to stage changes: {e}")))?;

        // Format the commit message with attribution
        let formatted_message = format_commit_message(message, files_modified);

        // Create commit
        let oid = self
            .git_manager
            .commit_changes(&repo, &formatted_message)
            .map_err(|e| ToolError::Mcp(format!("Failed to commit: {e}")))?;

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
    root: PathBuf,
}

impl GitBranchKit {
    pub fn new() -> Self {
        Self::with_root(arkavo_validation::current_workspace_root())
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            schema: ToolSchema {
                name: "git_branch".to_string(),
                aliases: Some(vec!["branch".to_string()]),
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
            .ok_or_else(|| ToolError::Mcp("Action is required".to_string()))?;

        let repo = safe_open_repo(&self.git_manager, &self.root, path)?;

        match action {
            "list" => {
                let branches = self
                    .git_manager
                    .list_branches(&repo)
                    .map_err(|e| ToolError::Mcp(format!("Failed to list branches: {e}")))?;
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
                    ToolError::Mcp("Branch name is required for create action".to_string())
                })?;
                self.git_manager
                    .create_branch(&repo, name)
                    .map_err(|e| ToolError::Mcp(format!("Failed to create branch: {e}")))?;
                Ok(json!({
                    "success": true,
                    "created": name
                }))
            }
            "switch" => {
                let name = params["name"].as_str().ok_or_else(|| {
                    ToolError::Mcp("Branch name is required for switch action".to_string())
                })?;
                self.git_manager
                    .checkout_branch(&repo, name)
                    .map_err(|e| ToolError::Mcp(format!("Failed to switch branch: {e}")))?;
                Ok(json!({
                    "success": true,
                    "switched_to": name
                }))
            }
            _ => Err(ToolError::Mcp(
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
    root: PathBuf,
}

impl GitLogKit {
    pub fn new() -> Self {
        Self::with_root(arkavo_validation::current_workspace_root())
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            schema: ToolSchema {
                name: "git_log".to_string(),
                aliases: Some(vec!["log".to_string()]),
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

        let repo = safe_open_repo(&self.git_manager, &self.root, path)?;

        let mut revwalk = repo
            .revwalk()
            .map_err(|e| ToolError::Mcp(format!("Failed to create revwalk: {e}")))?;
        revwalk
            .push_head()
            .map_err(|e| ToolError::Mcp(format!("Failed to push head: {e}")))?;

        let mut commits = Vec::new();
        for (i, oid) in revwalk.enumerate() {
            if i >= limit {
                break;
            }

            let oid = oid.map_err(|e| ToolError::Mcp(format!("Failed to get oid: {e}")))?;
            let commit = repo
                .find_commit(oid)
                .map_err(|e| ToolError::Mcp(format!("Failed to find commit: {e}")))?;

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
    root: PathBuf,
}

impl GitRemoteKit {
    pub fn new() -> Self {
        Self::with_root(arkavo_validation::current_workspace_root())
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            schema: ToolSchema {
                name: "git_remote".to_string(),
                aliases: Some(vec!["remote".to_string()]),
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
            .ok_or_else(|| ToolError::Mcp("Action is required".to_string()))?;
        let remote = params["remote"].as_str().unwrap_or("origin");

        let repo = safe_open_repo(&self.git_manager, &self.root, path)?;

        // Get current branch if not specified
        let branch = match params["branch"].as_str() {
            Some(b) => b.to_string(),
            None => self
                .git_manager
                .get_current_branch(&repo)
                .map_err(|e| ToolError::Mcp(format!("Failed to get current branch: {e}")))?,
        };

        match action {
            "fetch" => {
                self.git_manager
                    .fetch(&repo, remote)
                    .map_err(|e| ToolError::Mcp(format!("Failed to fetch: {e}")))?;
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
                    .map_err(|e| ToolError::Mcp(format!("Failed to pull: {e}")))?;
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
                    .map_err(|e| ToolError::Mcp(format!("Failed to push: {e}")))?;
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
                    .map_err(|e| ToolError::Mcp(format!("Failed to sync upstream: {e}")))?;
                self.git_manager
                    .publish(&repo)
                    .map_err(|e| ToolError::Mcp(format!("Failed to publish: {e}")))?;
                Ok(json!({
                    "success": true,
                    "action": "sync",
                    "remote": remote,
                    "branch": branch,
                    "message": format!("Successfully synced {} with {}", branch, remote)
                }))
            }
            _ => Err(ToolError::Mcp(
                "Invalid action. Use 'fetch', 'pull', 'push', or 'sync'".to_string(),
            )),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test needs block_on internally
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
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
        let commit_kit = GitCommitKit::with_root(temp_dir.path());
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

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_status_refuses_a_repository_outside_the_workspace() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        GitManager::new().init_repo(outside.path()).unwrap();
        let kit = GitStatusKit::with_root(ws.path());
        let err = kit
            .execute(json!({ "path": outside.path().to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn every_git_kit_refuses_a_repository_outside_the_workspace() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        GitManager::new().init_repo(outside.path()).unwrap();
        let path = outside.path().to_str().unwrap();
        let root = ws.path();
        let kits: Vec<(Box<dyn Tool>, Value)> = vec![
            (
                Box::new(GitDiffKit::with_root(root)),
                json!({ "path": path }),
            ),
            (
                Box::new(GitCommitKit::with_root(root)),
                json!({ "path": path, "message": "x" }),
            ),
            (
                Box::new(GitBranchKit::with_root(root)),
                json!({ "path": path, "action": "list" }),
            ),
            (
                Box::new(GitLogKit::with_root(root)),
                json!({ "path": path }),
            ),
            (
                Box::new(GitRemoteKit::with_root(root)),
                json!({ "path": path, "action": "fetch" }),
            ),
        ];
        for (kit, params) in kits {
            let err = kit.execute(params).await.unwrap_err();
            assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
        }
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_status_opens_the_workspace_repository_by_default_path() {
        let ws = TempDir::new().unwrap();
        let repo = git2::Repository::init(ws.path()).unwrap();
        // Status needs a born branch; commit through git2 rather than a kit so
        // a resolver regression can never make a test commit in a real checkout.
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        // No "path" defaults to ".", which must resolve against the kit's root.
        let status = GitStatusKit::with_root(ws.path())
            .execute(json!({}))
            .await
            .unwrap();
        assert!(status["branch"].is_string());
    }

    /// `outer` is a repository, `outer/ws` is a plain directory under it.
    fn nested_workspace() -> (TempDir, PathBuf) {
        let outer = TempDir::new().unwrap();
        let repo = git2::Repository::init(outer.path()).unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let ws = outer.path().join("ws");
        fs::create_dir(&ws).unwrap();
        fs::write(ws.join("note.txt"), "x").unwrap();
        (outer, ws)
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_status_refuses_an_enclosing_repository_above_the_workspace() {
        let (_outer, ws) = nested_workspace();
        let err = GitStatusKit::with_root(&ws)
            .execute(json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_commit_cannot_commit_into_an_enclosing_repository() {
        let (outer, ws) = nested_workspace();
        let repo = git2::Repository::open(outer.path()).unwrap();
        let head_before = repo.head().unwrap().target().unwrap();
        let err = GitCommitKit::with_root(&ws)
            .execute(json!({ "message": "escape" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
        assert_eq!(repo.head().unwrap().target().unwrap(), head_before);
        assert!(repo.index().unwrap().is_empty(), "nothing may be staged");
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_status_works_in_a_linked_worktree_whose_main_repository_is_inside_the_root() {
        let (outer, _ws) = nested_workspace();
        let repo = git2::Repository::open(outer.path()).unwrap();
        repo.worktree("wt", &outer.path().join("wt"), None).unwrap();
        // Root is `outer`: the worktree's `.git` file points at `outer/.git`,
        // which is inside it.
        let status = GitStatusKit::with_root(outer.path())
            .execute(json!({ "path": "wt" }))
            .await
            .unwrap();
        assert!(status["branch"].is_string());
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_status_refuses_a_linked_worktree_whose_main_repository_is_outside_the_root() {
        let (outer, _ws) = nested_workspace();
        let repo = git2::Repository::open(outer.path()).unwrap();
        let wt_dir = TempDir::new().unwrap();
        let wt_path = wt_dir.path().join("wt");
        repo.worktree("wt", &wt_path, None).unwrap();
        // The workspace's working directory is inside the root but its refs
        // and objects live in `outer/.git`, so the workspace is refused.
        let err = GitStatusKit::with_root(&wt_path)
            .execute(json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
    }

    /// Lay out a bare-bones `root/.git` that a test then points at an outer
    /// repository, the way a workspace-controlled repository could.
    fn crafted_git_dir(root: &Path) -> PathBuf {
        let git_dir = root.join(".git");
        fs::create_dir_all(git_dir.join("objects/info")).unwrap();
        fs::create_dir_all(git_dir.join("refs")).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/master\n").unwrap();
        fs::write(
            git_dir.join("config"),
            "[core]\n\trepositoryformatversion = 0\n",
        )
        .unwrap();
        git_dir
    }

    fn ref_and_object_snapshot(outer: &Path) -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                out.push(path.clone());
                if path.is_dir() {
                    walk(&path, out);
                }
            }
        }
        let mut all = Vec::new();
        walk(&outer.join(".git"), &mut all);
        all.sort();
        all
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_commit_refuses_a_workspace_git_dir_whose_commondir_is_an_outer_repository() {
        let (outer, ws) = nested_workspace();
        let git_dir = crafted_git_dir(&ws);
        let outer_git = fs::canonicalize(outer.path().join(".git")).unwrap();
        fs::write(git_dir.join("commondir"), outer_git.to_str().unwrap()).unwrap();
        fs::write(
            git_dir.join("gitdir"),
            git_dir.join("index").to_str().unwrap(),
        )
        .unwrap();
        let before = ref_and_object_snapshot(outer.path());
        let head_before = git2::Repository::open(outer.path())
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap();

        let err = GitCommitKit::with_root(&ws)
            .execute(json!({ "message": "pwn" }))
            .await
            .unwrap_err();

        assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
        assert_eq!(ref_and_object_snapshot(outer.path()), before);
        let outer_repo = git2::Repository::open(outer.path()).unwrap();
        assert_eq!(outer_repo.head().unwrap().target().unwrap(), head_before);
        assert!(outer_repo.find_reference("refs/heads/pwn").is_err());
    }

    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_status_refuses_any_object_alternates_entry() {
        let (outer, ws) = nested_workspace();
        GitManager::new().init_repo(&ws).unwrap();
        let outer_objects = fs::canonicalize(outer.path().join(".git/objects")).unwrap();
        let alternates = ws.join(".git/objects/info/alternates");
        fs::create_dir_all(alternates.parent().unwrap()).unwrap();
        fs::create_dir_all(ws.join("ws-objects")).unwrap();

        let outside_absolute = format!("{}\n", outer_objects.display());
        for entry in [
            outside_absolute.as_str(),
            "../../../.git/objects\n",
            "ws-objects\n",
            "../../ws-objects\n",
            "# shared\n../../ws-objects\n",
        ] {
            fs::write(&alternates, entry).unwrap();
            let err = GitStatusKit::with_root(&ws)
                .execute(json!({}))
                .await
                .unwrap_err();
            assert!(matches!(err, ToolError::PolicyDenied(_)), "{entry}: {err}");
        }

        // A file with only blanks and comments names nothing and is accepted.
        fs::write(&alternates, "\n# nothing\n").unwrap();
        let status = GitStatusKit::with_root(&ws).execute(json!({})).await;
        assert!(
            !matches!(status, Err(ToolError::PolicyDenied(_))),
            "{status:?}"
        );
    }

    /// A repository inside the workspace whose `.git/<name>` is a symlink into
    /// the outer repository's `.git/<name>`.
    #[cfg(unix)]
    fn workspace_with_symlinked_internal(name: &str) -> (TempDir, PathBuf) {
        let (outer, ws) = nested_workspace();
        GitManager::new().init_repo(&ws).unwrap();
        let inner = ws.join(".git").join(name);
        if inner.is_dir() {
            fs::remove_dir_all(&inner).unwrap();
        } else {
            fs::remove_file(&inner).unwrap();
        }
        let target = fs::canonicalize(outer.path().join(".git").join(name)).unwrap();
        std::os::unix::fs::symlink(target, &inner).unwrap();
        (outer, ws)
    }

    #[cfg(unix)]
    #[spec("MCP-011")]
    #[tokio::test]
    async fn git_kits_refuse_symlinked_objects_and_refs_and_leave_the_outer_repository_alone() {
        for name in ["objects", "refs"] {
            let (outer, ws) = workspace_with_symlinked_internal(name);
            let before = ref_and_object_snapshot(outer.path());

            let err = GitStatusKit::with_root(&ws)
                .execute(json!({}))
                .await
                .unwrap_err();
            assert!(matches!(err, ToolError::PolicyDenied(_)), "{name}: {err}");
            let err = GitCommitKit::with_root(&ws)
                .execute(json!({ "message": "pwn" }))
                .await
                .unwrap_err();
            assert!(matches!(err, ToolError::PolicyDenied(_)), "{name}: {err}");

            assert_eq!(ref_and_object_snapshot(outer.path()), before, "{name}");
            let outer_repo = git2::Repository::open(outer.path()).unwrap();
            assert!(outer_repo.find_reference("refs/heads/pwn").is_err());
        }
    }
}
