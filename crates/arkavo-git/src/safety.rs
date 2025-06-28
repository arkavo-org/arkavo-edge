use crate::{GitError, Result};
use git2::{Oid, Repository, ResetType};
use std::path::Path;
use std::process::Command;

/// A guard that ensures repository operations are atomic and can be rolled back
pub struct RepoGuard<'a> {
    repo: &'a Repository,
    initial_commit: Option<Oid>,
    validators: Vec<Box<dyn Fn() -> Result<()> + 'a>>,
}

impl<'a> RepoGuard<'a> {
    /// Create a new RepoGuard for the given repository
    pub fn new(repo: &'a Repository) -> Result<Self> {
        let initial_commit = repo.head().ok().and_then(|head| head.target());

        Ok(Self {
            repo,
            initial_commit,
            validators: Vec::new(),
        })
    }

    /// Add a pre-commit validator
    pub fn add_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn() -> Result<()> + 'a,
    {
        self.validators.push(Box::new(validator));
        self
    }

    /// Add cargo fmt check validator
    pub fn with_fmt_check(self) -> Self {
        self.add_validator(|| {
            let output = Command::new("cargo")
                .args(["fmt", "--", "--check"])
                .output()
                .map_err(|e| GitError::PreCommitFailed(format!("Failed to run cargo fmt: {e}")))?;

            if !output.status.success() {
                return Err(GitError::PreCommitFailed(
                    "Code formatting check failed. Run 'cargo fmt' to fix.".to_string(),
                ));
            }
            Ok(())
        })
    }

    /// Add cargo clippy check validator
    pub fn with_clippy_check(self) -> Self {
        self.add_validator(|| {
            let output = Command::new("cargo")
                .args(["clippy", "--", "-D", "warnings"])
                .output()
                .map_err(|e| {
                    GitError::PreCommitFailed(format!("Failed to run cargo clippy: {e}"))
                })?;

            if !output.status.success() {
                return Err(GitError::PreCommitFailed(
                    "Clippy check failed. Fix the warnings before committing.".to_string(),
                ));
            }
            Ok(())
        })
    }

    /// Add cargo test check validator
    pub fn with_test_check(self) -> Self {
        self.add_validator(|| {
            let output = Command::new("cargo")
                .args(["test", "--quiet"])
                .output()
                .map_err(|e| GitError::PreCommitFailed(format!("Failed to run cargo test: {e}")))?;

            if !output.status.success() {
                return Err(GitError::PreCommitFailed(
                    "Tests failed. Fix failing tests before committing.".to_string(),
                ));
            }
            Ok(())
        })
    }

    /// Execute a transaction with automatic rollback on failure
    pub fn transaction<F, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce(&Repository) -> Result<T>,
    {
        // Run pre-commit validators
        for validator in &self.validators {
            validator()?;
        }

        // Execute the operation
        match operation(self.repo) {
            Ok(result) => Ok(result),
            Err(e) => {
                // Rollback on error
                if let Some(commit_id) = self.initial_commit {
                    let commit = self.repo.find_commit(commit_id)?;
                    self.repo
                        .reset(&commit.into_object(), ResetType::Hard, None)?;
                }
                Err(e)
            }
        }
    }

    /// Commit changes with validation
    pub fn commit(&self, message: &str) -> Result<Oid> {
        use crate::GitManager;

        self.transaction(|repo| {
            let manager = GitManager::new();

            // Stage all changes
            manager.add_all(repo)?;

            // Create commit
            manager.commit_changes(repo, message)
        })
    }
}

/// Sanitize and validate a path to ensure it's within the repository
pub fn sanitize_repo_path(repo_root: &Path, requested_path: &Path) -> Result<std::path::PathBuf> {
    // Convert to absolute paths
    let repo_root = repo_root.canonicalize().map_err(GitError::Io)?;

    // If requested path is relative, make it relative to repo root
    let full_path = if requested_path.is_relative() {
        repo_root.join(requested_path)
    } else {
        requested_path.to_path_buf()
    };

    // Canonicalize to resolve .. and symlinks
    let canonical_path = full_path
        .canonicalize()
        .or_else(|_| {
            // If file doesn't exist yet, canonicalize the parent
            full_path
                .parent()
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid path")
                })?
                .canonicalize()
                .map(|parent| parent.join(full_path.file_name().unwrap_or_default()))
        })
        .map_err(GitError::Io)?;

    // Ensure the path is within the repository
    if !canonical_path.starts_with(&repo_root) {
        return Err(GitError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "Path '{}' is outside repository root",
                requested_path.display()
            ),
        )));
    }

    Ok(canonical_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_path_sanitization() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();

        // Create src directory for testing
        fs::create_dir(repo_root.join("src")).unwrap();

        // Test valid paths
        assert!(sanitize_repo_path(repo_root, Path::new("file.txt")).is_ok());
        assert!(sanitize_repo_path(repo_root, Path::new("src/main.rs")).is_ok());
        assert!(sanitize_repo_path(repo_root, repo_root.join("file.txt").as_path()).is_ok());

        // Test path traversal attempts
        assert!(sanitize_repo_path(repo_root, Path::new("../outside.txt")).is_err());
        assert!(sanitize_repo_path(repo_root, Path::new("../../etc/passwd")).is_err());

        // Test absolute path outside repo
        assert!(sanitize_repo_path(repo_root, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn test_repo_guard_rollback() {
        let temp_dir = TempDir::new().unwrap();
        let repo = Repository::init(temp_dir.path()).unwrap();

        // Set up git config for tests
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create initial commit
        fs::write(temp_dir.path().join("file1.txt"), "content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file1.txt")).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let initial_commit = repo
            .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();

        // Try operation that fails
        let guard = RepoGuard::new(&repo).unwrap();
        let result: Result<()> = guard.transaction(|repo| {
            // Make changes
            fs::write(temp_dir.path().join("file2.txt"), "new content").unwrap();
            let mut index = repo.index()?;
            index.add_path(Path::new("file2.txt"))?;
            index.write()?;

            // Simulate failure
            Err(GitError::CommitNotFound("Simulated error".to_string()))
        });

        assert!(result.is_err());

        // Verify rollback - file2.txt should not exist
        assert!(!temp_dir.path().join("file2.txt").exists());

        // Verify we're back at initial commit
        let head = repo.head().unwrap().target().unwrap();
        assert_eq!(head, initial_commit);
    }
}
