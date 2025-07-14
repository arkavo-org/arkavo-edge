pub mod attribution;
pub mod backend;
pub mod commit_message;
pub mod remote_fallback;
pub mod safety;

use backend::{Git2Backend, GitBackend, GitError, Result};
pub use git2::{Oid, Repository};
use std::path::Path;
use std::sync::Arc;

pub use backend::{DiffOptions, GitStatus};

pub struct GitManager {
    backend: Arc<dyn GitBackend>,
}

impl GitManager {
    pub fn new() -> Self {
        Self {
            backend: Arc::new(Git2Backend::new()),
        }
    }

    pub fn with_backend(backend: Arc<dyn GitBackend>) -> Self {
        Self { backend }
    }

    pub fn init_repo(&self, path: &Path) -> Result<()> {
        self.backend.init(path)?;
        Ok(())
    }

    pub fn open_repo(&self, path: &Path) -> Result<Repository> {
        self.backend.open(path)
    }

    pub fn status(&self, repo: &Repository) -> Result<GitStatus> {
        self.backend.status(repo)
    }

    pub fn add_files(&self, repo: &Repository, paths: &[&Path]) -> Result<()> {
        self.backend.add(repo, paths)
    }

    pub fn add_all(&self, repo: &Repository) -> Result<()> {
        self.backend.add_all(repo)
    }

    pub fn commit_changes(&self, repo: &Repository, message: &str) -> Result<Oid> {
        self.backend.commit(repo, message)
    }

    pub fn create_branch(&self, repo: &Repository, name: &str) -> Result<()> {
        self.backend.create_branch(repo, name)
    }

    pub fn checkout_branch(&self, repo: &Repository, name: &str) -> Result<()> {
        self.backend.checkout(repo, name)
    }

    pub fn list_branches(&self, repo: &Repository) -> Result<Vec<(String, bool)>> {
        self.backend.list_branches(repo)
    }

    pub fn diff(&self, repo: &Repository, options: &DiffOptions) -> Result<String> {
        self.backend.diff(repo, options)
    }

    pub fn fetch(&self, repo: &Repository, remote: &str) -> Result<()> {
        self.backend.fetch(repo, remote)
    }

    pub fn push(&self, repo: &Repository, remote: &str, branch: &str) -> Result<()> {
        self.backend.push(repo, remote, branch)
    }

    pub fn pull(&self, repo: &Repository, remote: &str, branch: &str) -> Result<()> {
        self.backend.pull(repo, remote, branch)
    }

    pub fn undo_last_commit(&self, repo: &Repository) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        if let Ok(parent) = head.parent(0) {
            self.backend.rollback(repo, parent.id())
        } else {
            Err(GitError::CommitNotFound(
                "No parent commit found".to_string(),
            ))
        }
    }

    pub fn get_current_branch(&self, repo: &Repository) -> Result<String> {
        self.backend.get_current_branch(repo)
    }

    pub fn sync_upstream(&self, repo: &Repository) -> Result<()> {
        let branch = self.get_current_branch(repo)?;
        self.backend.pull(repo, "origin", &branch)
    }

    pub fn publish(&self, repo: &Repository) -> Result<()> {
        let branch = self.get_current_branch(repo)?;
        self.backend.push(repo, "origin", &branch)
    }

    /// Commit changes with an auto-generated message
    pub fn auto_commit(&self, repo: &Repository) -> Result<Oid> {
        use crate::commit_message::CommitMessageGenerator;

        // Stage all changes
        self.add_all(repo)?;

        // Generate commit message
        let generator = CommitMessageGenerator::new();
        let message = generator.generate(repo)?;

        // Add AI-generated suffix
        let full_message = format!("{message}\n\n[AI-generated]");

        // Create commit
        self.commit_changes(repo, &full_message)
    }
}

impl Default for GitManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init_repo() -> Result<()> {
    let manager = GitManager::new();
    manager.init_repo(Path::new("."))
}

pub fn create_branch(name: &str) -> Result<()> {
    let manager = GitManager::new();
    let repo = manager.open_repo(Path::new("."))?;
    manager.create_branch(&repo, name)
}

pub fn commit_changes(message: &str) -> Result<()> {
    let manager = GitManager::new();
    let repo = manager.open_repo(Path::new("."))?;
    manager.add_all(&repo)?;
    manager.commit_changes(&repo, message)?;
    Ok(())
}

pub fn undo_last_commit() -> Result<()> {
    let manager = GitManager::new();
    let repo = manager.open_repo(Path::new("."))?;
    manager.undo_last_commit(&repo)
}
