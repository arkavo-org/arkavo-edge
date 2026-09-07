use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{CodexConfig, Sandbox};

/// Root of the host's own state tree, `$HOME/.arkavo`.
///
/// Two rules need it — the workspace exemption below and the thread
/// rendezvous — so it is resolved in one place. It is canonicalized when it
/// already exists, because the workspace it is compared against is canonical
/// and a home directory reached through a symlink would otherwise never match.
fn arkavo_root() -> Result<PathBuf> {
    let root = std::env::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory for Codex session state"))?
        .join(".arkavo");
    Ok(root.canonicalize().unwrap_or(root))
}

/// Create a directory only this account can enter.
///
/// It holds one user's session locks; no other account has business opening
/// them, and the mode is what keeps that true when `$HOME` is group-readable.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Whether session state may live at `parent` for a worker granted `workspace`.
///
/// Codex is granted the workspace, so state inside that tree is state the model
/// itself could rewrite. The single exception is the host's own `.arkavo` tree:
/// a host whose workspace is `$HOME` — or `/` — contains it by accident of the
/// grant being wide, not because the grant was aimed at the state directory.
/// The exception is void as soon as the workspace *is* the `.arkavo` root, or
/// sits inside it, because then the grant does name that directory.
fn state_location_is_permitted(parent: &Path, workspace: &Path, arkavo_root: &Path) -> bool {
    if !parent.starts_with(workspace) {
        return true;
    }
    let state_is_host_owned = parent.starts_with(arkavo_root);
    let workspace_merely_contains_the_root =
        arkavo_root != workspace && arkavo_root.starts_with(workspace);
    state_is_host_owned && workspace_merely_contains_the_root
}

/// The home directory is consulted only when the state does fall inside the
/// workspace, so a host with no resolvable home can still keep state elsewhere.
fn ensure_state_outside_workspace(parent: &Path, workspace: &Path) -> Result<()> {
    if !parent.starts_with(workspace) {
        return Ok(());
    }
    let root = arkavo_root()?;
    ensure!(
        state_location_is_permitted(parent, workspace, &root),
        "Session state must be outside the worker workspace"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBinding {
    pub workspace: PathBuf,
    pub agent_id: String,
    pub model: String,
    pub sandbox: Sandbox,
    pub thread_id: Option<String>,
    /// A started attempt without fully recorded usage requires host reconciliation.
    pub accounting_incomplete: bool,
}

pub(crate) struct Store {
    path: PathBuf,
    lock: File,
    /// Held while this worker owns the Codex thread named in the binding.
    /// The state-file lock alone protects a path, and two copies of one state
    /// file are two paths naming one remote session.
    thread_lock: Option<File>,
    pub(crate) binding: SessionBinding,
}

impl Drop for Store {
    fn drop(&mut self) {
        // Explicit unlock also releases the lease if a concurrent process spawn
        // briefly inherited the descriptor before exec closed it.
        self.lock.unlock().ok();
        if let Some(lock) = &self.thread_lock {
            lock.unlock().ok();
        }
    }
}

impl Store {
    pub(crate) fn open(path: &Path, config: &CodexConfig) -> Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("State requires a parent directory"))?
            .canonicalize()?;
        ensure_state_outside_workspace(&parent, &config.workspace)?;
        let path = parent.join(
            path.file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid state path"))?,
        );
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("lock"))?;
        lock.try_lock()
            .map_err(|_| anyhow::anyhow!("Session is already owned by another worker"))?;
        let expected = SessionBinding {
            workspace: config.workspace.clone(),
            agent_id: config.agent_id.clone(),
            model: config.model.clone(),
            sandbox: config.sandbox,
            thread_id: None,
            accounting_incomplete: false,
        };
        let binding = match std::fs::read(&path) {
            Ok(bytes) => {
                let saved: SessionBinding = serde_json::from_slice(&bytes)?;
                ensure!(
                    saved.workspace == expected.workspace
                        && saved.agent_id == expected.agent_id
                        && saved.model == expected.model
                        && saved.sandbox == expected.sandbox,
                    "Session workspace, identity, model or permissions changed"
                );
                if let Some(id) = &saved.thread_id {
                    uuid::Uuid::parse_str(id)?;
                }
                saved
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => expected,
            Err(e) => return Err(e.into()),
        };
        let thread_lock = binding
            .thread_id
            .as_deref()
            .map(Self::lock_thread)
            .transpose()?;
        let store = Self {
            path,
            lock,
            thread_lock,
            binding,
        };
        store.save()?;
        Ok(store)
    }

    /// Rendezvous file for one Codex thread, under the host's own state tree.
    ///
    /// A world-writable temporary directory lets any local account pre-plant a
    /// symlink at this well-known name and redirect the open onto a file of its
    /// choosing. Codex authenticates per user, so a thread is owned per user
    /// rather than machine-wide and a private directory gives up nothing.
    fn thread_lock_path(id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id)?;
        let dir = arkavo_root()?.join("codex").join("threads");
        create_private_dir(&dir)?;
        Ok(dir.join(format!("thread-{id}.lock")))
    }

    /// Claim a Codex thread for this process. The lock file is never unlinked:
    /// unlink-then-unlock would let a waiter on the old inode and a new opener
    /// on the replacement both believe they own the thread.
    fn lock_thread(id: &str) -> Result<File> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(Self::thread_lock_path(id)?)?;
        lock.try_lock()
            .map_err(|_| anyhow::anyhow!("Codex thread is already owned by another worker"))?;
        Ok(lock)
    }

    /// Record the thread Codex reported, taking ownership of it first.
    pub(crate) fn bind_thread(&mut self, id: &str) -> Result<()> {
        if self.binding.thread_id.as_deref() == Some(id) {
            return Ok(());
        }
        ensure!(
            self.binding.thread_id.is_none(),
            "Codex reported a different thread for this session"
        );
        self.thread_lock = Some(Self::lock_thread(id)?);
        self.binding.thread_id = Some(id.to_owned());
        self.save()
    }

    pub(crate) fn save(&self) -> Result<()> {
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(&serde_json::to_vec(&self.binding)?)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            std::fs::remove_file(temporary).ok();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule is exercised against synthetic paths rather than the real
    /// `$HOME`: a fixture cannot make the home directory a Codex workspace
    /// without letting the worker write into it, and mutating `$HOME` would
    /// race every other test in the binary.
    #[test]
    fn host_state_root_is_the_only_workspace_exemption() {
        let root = Path::new("/home/u/.arkavo");
        let state = "/home/u/.arkavo/codex";
        for (parent, workspace, permitted, case) in [
            // A host started from the home directory keeps its tools: the
            // dotfile tree is the host's, not the target of the grant.
            (state, "/home/u", true, "workspace is the home directory"),
            (state, "/", true, "workspace is the filesystem root"),
            // The ordinary case: state was never inside the workspace.
            (
                state,
                "/home/u/project",
                true,
                "workspace below the home directory",
            ),
            // The grant now names the state tree itself, so the exemption ends.
            (
                state,
                "/home/u/.arkavo",
                false,
                "workspace contains the state root",
            ),
            (
                state,
                "/home/u/.arkavo/codex",
                false,
                "workspace is the state root",
            ),
            // Nothing about the exemption relaxes the ordinary refusal.
            (
                "/home/u/project/.state",
                "/home/u/project",
                false,
                "state inside an ordinary workspace",
            ),
            (
                "/home/u/project",
                "/home/u/project",
                false,
                "state directly in the workspace",
            ),
        ] {
            assert_eq!(
                state_location_is_permitted(Path::new(parent), Path::new(workspace), root),
                permitted,
                "{case}"
            );
        }
    }

    /// A prefix match is not a path match: `/home/user2` must not count as
    /// being inside `/home/u`.
    #[test]
    fn sibling_directories_sharing_a_name_prefix_are_not_nested() {
        assert!(state_location_is_permitted(
            Path::new("/home/user2/.arkavo/codex"),
            Path::new("/home/u"),
            Path::new("/home/user2/.arkavo"),
        ));
    }

    /// The rendezvous moved out of the shared temporary directory, where any
    /// local account could pre-plant a symlink at the well-known name.
    #[test]
    fn thread_locks_live_under_the_private_host_state_root() {
        let id = uuid::Uuid::new_v4().to_string();
        let path = Store::thread_lock_path(&id).expect("thread lock path");
        assert!(path.starts_with(arkavo_root().expect("state root")));
        assert!(!path.starts_with(std::env::temp_dir()));
        assert!(path.ends_with(format!("thread-{id}.lock")));
    }

    #[test]
    fn thread_locks_require_a_codex_thread_identifier() {
        assert!(Store::thread_lock_path("../escape").is_err());
    }
}
