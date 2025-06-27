use git2::{Oid, Repository, Status};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitError {
    #[error("Git operation failed: {0}")]
    Git(#[from] git2::Error),
    #[error("IO operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Repository not found at path: {0}")]
    RepoNotFound(String),
    #[error("Branch not found: {0}")]
    BranchNotFound(String),
    #[error("Remote not found: {0}")]
    RemoteNotFound(String),
    #[error("Commit not found: {0}")]
    CommitNotFound(String),
    #[error("Working directory has uncommitted changes")]
    DirtyWorkingDirectory,
    #[error("Merge conflict detected")]
    MergeConflict,
    #[error("Pre-commit validation failed: {0}")]
    PreCommitFailed(String),
}

pub type Result<T> = std::result::Result<T, GitError>;

pub struct GitStatus {
    pub modified: Vec<String>,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
    pub renamed: Vec<(String, String)>,
    pub untracked: Vec<String>,
    pub conflicted: Vec<String>,
}

pub struct DiffOptions {
    pub staged: bool,
    pub unstaged: bool,
    pub cached: bool,
    pub context_lines: u32,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            staged: false,
            unstaged: true,
            cached: false,
            context_lines: 3,
        }
    }
}

pub trait GitBackend: Send + Sync {
    fn init(&self, path: &Path) -> Result<Repository>;
    fn open(&self, path: &Path) -> Result<Repository>;
    fn status(&self, repo: &Repository) -> Result<GitStatus>;
    fn add(&self, repo: &Repository, paths: &[&Path]) -> Result<()>;
    fn add_all(&self, repo: &Repository) -> Result<()>;
    fn commit(&self, repo: &Repository, message: &str) -> Result<Oid>;
    fn create_branch(&self, repo: &Repository, name: &str) -> Result<()>;
    fn checkout(&self, repo: &Repository, name: &str) -> Result<()>;
    fn list_branches(&self, repo: &Repository) -> Result<Vec<(String, bool)>>;
    fn diff(&self, repo: &Repository, options: &DiffOptions) -> Result<String>;
    fn fetch(&self, repo: &Repository, remote: &str) -> Result<()>;
    fn push(&self, repo: &Repository, remote: &str, branch: &str) -> Result<()>;
    fn pull(&self, repo: &Repository, remote: &str, branch: &str) -> Result<()>;
    fn rollback(&self, repo: &Repository, commit: Oid) -> Result<()>;
    fn get_current_branch(&self, repo: &Repository) -> Result<String>;
    fn get_commit_message(&self, repo: &Repository, oid: Oid) -> Result<String>;
}

pub struct Git2Backend;

impl Git2Backend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Git2Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl GitBackend for Git2Backend {
    fn init(&self, path: &Path) -> Result<Repository> {
        let repo = Repository::init(path)?;

        // Set default branch to "main"
        repo.config()?.set_str("init.defaultBranch", "main")?;

        Ok(repo)
    }

    fn open(&self, path: &Path) -> Result<Repository> {
        Repository::discover(path).or_else(|_| {
            Repository::open(path).map_err(|_| GitError::RepoNotFound(path.display().to_string()))
        })
    }

    fn status(&self, repo: &Repository) -> Result<GitStatus> {
        let statuses = repo.statuses(None)?;
        let mut status = GitStatus {
            modified: vec![],
            added: vec![],
            deleted: vec![],
            renamed: vec![],
            untracked: vec![],
            conflicted: vec![],
        };

        for entry in statuses.iter() {
            if let Some(path) = entry.path() {
                let s = entry.status();
                if s.contains(Status::WT_MODIFIED) || s.contains(Status::INDEX_MODIFIED) {
                    status.modified.push(path.to_string());
                } else if s.contains(Status::WT_NEW) || s.contains(Status::INDEX_NEW) {
                    status.added.push(path.to_string());
                } else if s.contains(Status::WT_DELETED) || s.contains(Status::INDEX_DELETED) {
                    status.deleted.push(path.to_string());
                } else if s.contains(Status::WT_RENAMED) || s.contains(Status::INDEX_RENAMED) {
                    if let Some(delta) = entry.head_to_index() {
                        if let (Some(old), Some(new)) =
                            (delta.old_file().path(), delta.new_file().path())
                        {
                            status
                                .renamed
                                .push((old.display().to_string(), new.display().to_string()));
                        }
                    }
                } else if s.contains(Status::CONFLICTED) {
                    status.conflicted.push(path.to_string());
                } else if s.is_wt_new() && !s.is_index_new() {
                    status.untracked.push(path.to_string());
                }
            }
        }

        Ok(status)
    }

    fn add(&self, repo: &Repository, paths: &[&Path]) -> Result<()> {
        let mut index = repo.index()?;
        for path in paths {
            index.add_path(path)?;
        }
        index.write()?;
        Ok(())
    }

    fn add_all(&self, repo: &Repository) -> Result<()> {
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        Ok(())
    }

    fn commit(&self, repo: &Repository, message: &str) -> Result<Oid> {
        let mut index = repo.index()?;
        let oid = index.write_tree()?;
        let tree = repo.find_tree(oid)?;
        let sig = repo.signature()?;
        let parent_commit = match repo.head() {
            Ok(head) => Some(head.peel_to_commit()?),
            Err(_) => None,
        };
        let parents = parent_commit.as_ref().map(|c| vec![c]).unwrap_or_default();

        Ok(repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?)
    }

    fn create_branch(&self, repo: &Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    fn checkout(&self, repo: &Repository, name: &str) -> Result<()> {
        let obj = repo.revparse_single(&format!("refs/heads/{name}"))?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head(&format!("refs/heads/{name}"))?;
        Ok(())
    }

    fn list_branches(&self, repo: &Repository) -> Result<Vec<(String, bool)>> {
        let mut branches = vec![];
        let head = repo.head()?;
        let current_branch = head.shorthand().unwrap_or("");

        for branch in repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = branch?;
            if let Some(name) = branch.name()? {
                branches.push((name.to_string(), name == current_branch));
            }
        }

        Ok(branches)
    }

    fn diff(&self, repo: &Repository, options: &DiffOptions) -> Result<String> {
        let mut diff_options = git2::DiffOptions::new();
        diff_options.context_lines(options.context_lines);

        let diff = if options.staged {
            let head = repo.head()?.peel_to_tree()?;
            let index = repo.index()?;
            repo.diff_tree_to_index(Some(&head), Some(&index), Some(&mut diff_options))?
        } else if options.cached {
            let head = repo.head()?.peel_to_tree()?;
            repo.diff_tree_to_tree(Some(&head), None, Some(&mut diff_options))?
        } else {
            repo.diff_index_to_workdir(None, Some(&mut diff_options))?
        };

        let mut output = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            output.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
            true
        })?;

        Ok(output)
    }

    fn fetch(&self, repo: &Repository, remote: &str) -> Result<()> {
        // Check if we need to use fallback for HTTPS
        if let Ok(remote_obj) = repo.find_remote(remote) {
            if let Some(url) = remote_obj.url() {
                if crate::remote_fallback::is_https_url(url)
                    || crate::remote_fallback::is_ssh_url(url)
                {
                    // Use system git for HTTPS/SSH operations
                    if let Some(workdir) = repo.workdir() {
                        return crate::remote_fallback::git_fallback_fetch(workdir, remote);
                    }
                }
            }
        }

        // Try native git2 fetch (will work for local/file URLs)
        let mut remote = repo.find_remote(remote)?;
        remote.fetch(&[] as &[&str], None, None)?;
        Ok(())
    }

    fn push(&self, repo: &Repository, remote: &str, branch: &str) -> Result<()> {
        // Check if we need to use fallback for HTTPS
        if let Ok(remote_obj) = repo.find_remote(remote) {
            if let Some(url) = remote_obj.url() {
                if crate::remote_fallback::is_https_url(url)
                    || crate::remote_fallback::is_ssh_url(url)
                {
                    // Use system git for HTTPS/SSH operations
                    if let Some(workdir) = repo.workdir() {
                        return crate::remote_fallback::git_fallback_push(workdir, remote, branch);
                    }
                }
            }
        }

        // Try native git2 push (will work for local/file URLs)
        let mut remote = repo.find_remote(remote)?;
        remote.push(&[format!("refs/heads/{branch}")], None)?;
        Ok(())
    }

    fn pull(&self, repo: &Repository, remote: &str, branch: &str) -> Result<()> {
        // Check if we need to use fallback for HTTPS
        if let Ok(remote_obj) = repo.find_remote(remote) {
            if let Some(url) = remote_obj.url() {
                if crate::remote_fallback::is_https_url(url)
                    || crate::remote_fallback::is_ssh_url(url)
                {
                    // Use system git for HTTPS/SSH operations
                    if let Some(workdir) = repo.workdir() {
                        return crate::remote_fallback::git_fallback_pull(workdir, remote, branch);
                    }
                }
            }
        }

        // Native implementation for local/file URLs
        self.fetch(repo, remote)?;

        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        let analysis = repo.merge_analysis(&[&fetch_commit])?;

        if analysis.0.is_up_to_date() {
            return Ok(());
        } else if analysis.0.is_fast_forward() {
            let refname = format!("refs/heads/{branch}");
            let mut reference = repo.find_reference(&refname)?;
            reference.set_target(fetch_commit.id(), "Fast-forward")?;
            repo.set_head(&refname)?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        } else {
            return Err(GitError::MergeConflict);
        }

        Ok(())
    }

    fn rollback(&self, repo: &Repository, commit: Oid) -> Result<()> {
        let commit = repo.find_commit(commit)?;
        repo.reset(&commit.into_object(), git2::ResetType::Hard, None)?;
        Ok(())
    }

    fn get_current_branch(&self, repo: &Repository) -> Result<String> {
        let head = repo.head()?;
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    }

    fn get_commit_message(&self, repo: &Repository, oid: Oid) -> Result<String> {
        let commit = repo.find_commit(oid)?;
        Ok(commit.message().unwrap_or("").to_string())
    }
}
