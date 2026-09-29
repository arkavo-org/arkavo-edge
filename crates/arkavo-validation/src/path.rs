use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub enum PathValidationError {
    TraversalAttempt(String),
    OutsideRoot { path: PathBuf, root: PathBuf },
    InvalidPath(String),
}

impl std::fmt::Display for PathValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathValidationError::TraversalAttempt(p) => {
                write!(f, "Path traversal attempt: {p}")
            }
            PathValidationError::OutsideRoot { path, root } => {
                write!(
                    f,
                    "Path {} is outside allowed root {}",
                    path.display(),
                    root.display()
                )
            }
            PathValidationError::InvalidPath(p) => write!(f, "Invalid path: {p}"),
        }
    }
}

impl std::error::Error for PathValidationError {}

/// Validate that a path has no traversal patterns (`..` or `~`).
/// Resolves relative paths against the given base directory.
/// Does NOT enforce containment within the base — use `validate_path_within_root` for that.
/// Suitable when the caller intentionally allows access to any absolute path.
pub fn validate_no_traversal(base: &Path, path: &str) -> Result<PathBuf, PathValidationError> {
    let path_obj = Path::new(path);
    let path_str = path_obj.to_string_lossy();

    if path_str.contains("..") {
        return Err(PathValidationError::TraversalAttempt(path_str.to_string()));
    }
    if path_str.contains('~') {
        return Err(PathValidationError::TraversalAttempt(path_str.to_string()));
    }

    if path_obj.is_relative() {
        Ok(base.join(path_obj))
    } else {
        Ok(path_obj.to_path_buf())
    }
}

/// Validate that a path stays within the given root directory.
/// Rejects `..` components, `~`, and paths that resolve outside root.
///
/// NOTE: This is pattern-based and does NOT resolve symlinks. If the
/// filesystem may contain attacker-controlled symlinks, call
/// `std::fs::canonicalize()` on the result before accessing the file.
pub fn validate_path_within_root(root: &Path, path: &str) -> Result<PathBuf, PathValidationError> {
    let path_obj = Path::new(path);

    // Reject explicit traversal patterns
    let path_str = path_obj.to_string_lossy();
    if path_str.contains("..") {
        return Err(PathValidationError::TraversalAttempt(path_str.to_string()));
    }
    if path_str.contains('~') {
        return Err(PathValidationError::TraversalAttempt(path_str.to_string()));
    }

    // Build absolute path
    let abs_path = if path_obj.is_relative() {
        root.join(path_obj)
    } else {
        path_obj.to_path_buf()
    };

    // Verify it starts with the root
    if !abs_path.starts_with(root) {
        return Err(PathValidationError::OutsideRoot {
            path: abs_path,
            root: root.to_path_buf(),
        });
    }

    Ok(abs_path)
}

/// Resolve `requested` (absolute, or relative to `root`) to an absolute path,
/// canonicalising every existing ancestor (thus following symlinks) and
/// appending any not-yet-existing tail literally, then require the result to
/// live inside the canonicalised `root`. Refuses traversal, symlink escape,
/// dangling-symlink ancestors, and, on Windows, drive-relative prefixes.
/// Fail-closed: any resolution error is a refusal, never a pass-through.
pub fn resolve_within_root(root: &Path, requested: &str) -> Result<PathBuf, PathValidationError> {
    if requested.is_empty() {
        return Err(PathValidationError::InvalidPath("empty path".into()));
    }
    let given = Path::new(requested);
    reject_drive_relative(given)?;

    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| PathValidationError::InvalidPath(format!("workspace root: {e}")))?;

    let candidate = if given.is_absolute() {
        given.to_path_buf()
    } else {
        canonical_root.join(given)
    };

    let resolved = resolve_existing_prefix(&candidate)
        .map_err(|e| PathValidationError::InvalidPath(e.to_string()))?;

    if resolved.starts_with(&canonical_root) {
        Ok(resolved)
    } else {
        Err(PathValidationError::OutsideRoot {
            path: resolved,
            root: canonical_root,
        })
    }
}

/// The zero-config workspace root: this process's working directory,
/// canonicalised once. Each agent process runs with its workspace as its
/// working directory (SwarmKit `isolation.sandbox: process`), the same
/// convention the taint guard's `DestinationPolicy` uses. An unreadable
/// working directory yields an empty root that nothing resolves inside, so
/// every confined call is refused rather than let through.
pub fn current_workspace_root() -> PathBuf {
    std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .unwrap_or_default()
}

#[cfg(windows)]
fn reject_drive_relative(p: &Path) -> Result<(), PathValidationError> {
    use std::path::Prefix;
    let mut comps = p.components();
    if let Some(Component::Prefix(prefix)) = comps.next() {
        // `C:foo` (a disk prefix not followed by a root) resolves against the
        // drive's hidden per-drive working directory, which no root check sees.
        if matches!(prefix.kind(), Prefix::Disk(_))
            && !matches!(comps.next(), Some(Component::RootDir))
        {
            return Err(PathValidationError::InvalidPath(
                "drive-relative path is not allowed".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn reject_drive_relative(_p: &Path) -> Result<(), PathValidationError> {
    Ok(())
}

fn resolve_existing_prefix(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => out.push(component.as_os_str()),
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(_) => {
                out.push(component.as_os_str());
                match std::fs::canonicalize(&out) {
                    Ok(resolved) => out = resolved,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // NotFound also covers a dangling symlink; that must
                        // fail closed rather than read as a fresh file, so
                        // probe the link itself.
                        match std::fs::symlink_metadata(&out) {
                            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {}
                            _ => return Err(e),
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! Unit tests for path validation and traversal prevention.
    //!
    //! ## Spec Coverage
    //! - [specs/arkavo-edge/validation.spec.yaml](VAL-001): Path traversal prevention
    //! - [specs/arkavo-edge/validation.spec.yaml](VAL-002): Path within root enforcement
    //! - [specs/arkavo-edge/validation.spec.yaml](VAL-008): Resolve a path within the workspace root

    use super::*;
    use arkavo_test_macros::spec;

    #[spec("VAL-002")]
    #[test]
    fn test_valid_relative_path() {
        let root = Path::new("/workspace");
        let result = validate_path_within_root(root, "src/main.rs");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/workspace/src/main.rs"));
    }

    #[spec("VAL-001")]
    #[test]
    fn test_traversal_blocked() {
        let root = Path::new("/workspace");
        assert!(validate_path_within_root(root, "../etc/passwd").is_err());
        assert!(validate_path_within_root(root, "src/../../etc").is_err());
    }

    #[spec("VAL-001")]
    #[test]
    fn test_tilde_blocked() {
        let root = Path::new("/workspace");
        assert!(validate_path_within_root(root, "~/secret").is_err());
    }

    #[spec("VAL-002")]
    #[test]
    fn test_absolute_outside_root() {
        let root = Path::new("/workspace");
        assert!(validate_path_within_root(root, "/etc/passwd").is_err());
    }

    #[spec("VAL-002")]
    #[test]
    fn test_absolute_inside_root() {
        let root = Path::new("/workspace");
        let result = validate_path_within_root(root, "/workspace/src/lib.rs");
        assert!(result.is_ok());
    }

    #[spec("VAL-001")]
    #[test]
    fn test_no_traversal_allows_absolute() {
        let base = Path::new("/workspace");
        let result = validate_no_traversal(base, "/tmp/file.txt");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/tmp/file.txt"));
    }

    #[spec("VAL-001")]
    #[test]
    fn test_no_traversal_blocks_dotdot() {
        let base = Path::new("/workspace");
        assert!(validate_no_traversal(base, "../etc/passwd").is_err());
    }

    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_allows_new_nested_target() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let got = resolve_within_root(&root, "a/b/new.txt").unwrap();
        assert!(got.starts_with(&root));
        assert!(got.ends_with("a/b/new.txt"));
    }

    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_rejects_absolute_escape() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("ws")).unwrap();
        let root = std::fs::canonicalize(tmp.path().join("ws")).unwrap();
        // A sibling absolute path outside the workspace.
        let outside = root.parent().unwrap().join("secret.txt");
        std::fs::write(&outside, b"k").unwrap();
        assert!(matches!(
            resolve_within_root(&root, outside.to_str().unwrap()),
            Err(PathValidationError::OutsideRoot { .. })
        ));
    }

    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_rejects_parent_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        assert!(resolve_within_root(&root, "../escape.txt").is_err());
        // A literal filename containing "..", which the old substring test wrongly
        // rejected, is now accepted because it stays inside the root.
        assert!(
            resolve_within_root(&root, "weird..name")
                .unwrap()
                .starts_with(&root)
        );
    }

    #[cfg(unix)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_refuses_symlink_escape() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("ws")).unwrap();
        let root = std::fs::canonicalize(tmp.path().join("ws")).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(outside.join("child")).unwrap();
        symlink(outside.join("child"), root.join("link")).unwrap();
        // Reading/writing *through* the symlink escapes the workspace → refused.
        assert!(matches!(
            resolve_within_root(&root, "link/loot.txt"),
            Err(PathValidationError::OutsideRoot { .. })
        ));
        // A dangling symlink ancestor cannot establish containment → fail closed.
        symlink(outside.join("missing"), root.join("dangling")).unwrap();
        assert!(resolve_within_root(&root, "dangling/x").is_err());
    }

    #[spec("VAL-008")]
    #[test]
    fn current_workspace_root_is_the_canonical_working_directory() {
        let cwd = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        assert_eq!(current_workspace_root(), cwd);
    }

    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_fails_closed_on_empty_path_and_unreadable_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        assert!(matches!(
            resolve_within_root(&root, ""),
            Err(PathValidationError::InvalidPath(_))
        ));
        // The empty root `current_workspace_root` yields for an unreadable cwd.
        assert!(matches!(
            resolve_within_root(Path::new(""), "x.txt"),
            Err(PathValidationError::InvalidPath(_))
        ));
        assert!(resolve_within_root(&root.join("missing-root"), "x.txt").is_err());
    }

    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_accepts_absolute_inside_and_contained_parent_components() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let inside = root.join("src").join("lib.rs");
        assert_eq!(
            resolve_within_root(&root, inside.to_str().unwrap()).unwrap(),
            inside
        );
        assert_eq!(
            resolve_within_root(&root, "src/../src/./lib.rs").unwrap(),
            inside
        );
        // A sibling that merely shares the root's name as a prefix is outside.
        let sibling = format!("{}-other/x", root.display());
        assert!(matches!(
            resolve_within_root(&root, &sibling),
            Err(PathValidationError::OutsideRoot { .. })
        ));
    }

    #[cfg(unix)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_follows_symlinks_that_stay_inside() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
        assert_eq!(
            resolve_within_root(&root, "alias/new.txt").unwrap(),
            root.join("real").join("new.txt")
        );
    }

    #[cfg(windows)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_rejects_drive_relative_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        assert!(matches!(
            resolve_within_root(&root, "C:foo"),
            Err(PathValidationError::InvalidPath(_))
        ));
    }

    #[cfg(windows)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_rejects_rooted_path_without_drive_that_leaves_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        assert!(resolve_within_root(&root, "\\Windows\\System32").is_err());
    }
}
