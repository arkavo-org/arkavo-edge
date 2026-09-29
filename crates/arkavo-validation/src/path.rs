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

    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| PathValidationError::InvalidPath(format!("workspace root: {e}")))?;
    // Before any filesystem access to the request: canonicalising a UNC or
    // device path would open it, handing a hostile host our credentials.
    reject_foreign_volume(given, &canonical_root)?;

    let candidate = if given.is_absolute() {
        given.to_path_buf()
    } else {
        canonical_root.join(given)
    };

    let resolved = resolve_through_existing_ancestors(&candidate)
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

/// Only a plain or verbatim drive on the root's own drive may carry a prefix.
/// `C:foo` resolves against the drive's hidden per-drive working directory,
/// which no root check sees; UNC, device and `GLOBALROOT` prefixes name
/// resources other than this volume.
#[cfg(windows)]
fn reject_foreign_volume(p: &Path, canonical_root: &Path) -> Result<(), PathValidationError> {
    let mut comps = p.components();
    let Some(Component::Prefix(prefix)) = comps.next() else {
        return Ok(());
    };
    let refuse = |why: &str| Err(PathValidationError::InvalidPath(why.into()));
    if !matches!(comps.next(), Some(Component::RootDir)) {
        return refuse("drive-relative path is not allowed");
    }
    let drive = disk_letter(prefix.kind());
    let root_drive = match canonical_root.components().next() {
        Some(Component::Prefix(root_prefix)) => disk_letter(root_prefix.kind()),
        _ => None,
    };
    match (drive, root_drive) {
        (Some(d), Some(r)) if d == r => Ok(()),
        _ => refuse("path is not on the workspace drive"),
    }
}

#[cfg(windows)]
fn disk_letter(kind: std::path::Prefix<'_>) -> Option<u8> {
    use std::path::Prefix;
    match kind {
        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some(d.to_ascii_uppercase()),
        _ => None,
    }
}

#[cfg(not(windows))]
fn reject_foreign_volume(_p: &Path, _canonical_root: &Path) -> Result<(), PathValidationError> {
    Ok(())
}

/// Canonicalise `absolute` component by component, following symlinks in every
/// existing ancestor and appending the not-yet-existing tail literally, so a
/// new file or directory can be planned without the symlink games that a
/// lexical check misses. `..` is applied after the ancestors before it are
/// resolved, since a symlink changes what its parent is.
///
/// Fail-closed: a dangling symlink (its target does not exist) and any
/// unreadable ancestor are errors, because neither can establish where a write
/// would land. A relative path is also an error: this function has no
/// notion of a base directory, and silently using the process working
/// directory would make containment depend on ambient state. Callers that
/// want that convention join the working directory themselves.
pub fn resolve_through_existing_ancestors(absolute: &Path) -> std::io::Result<PathBuf> {
    if !absolute.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path {} is not absolute", absolute.display()),
        ));
    }
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
                            Ok(_) => {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    format!(
                                        "{} is a dangling symlink: the symlink target does not exist and was refused",
                                        out.display()
                                    ),
                                ));
                            }
                            Err(_) => return Err(e),
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
    fn resolve_within_root_fails_closed_on_empty_path_and_missing_or_empty_root() {
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

    #[cfg(unix)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_refuses_dangling_leaf_symlink_with_a_dedicated_message() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("ws")).unwrap();
        let root = std::fs::canonicalize(tmp.path().join("ws")).unwrap();
        let outside_target = tmp.path().join("outside").join("not-created.txt");
        std::os::unix::fs::symlink(&outside_target, root.join("dangling")).unwrap();
        // Writing to the leaf would create the file outside the workspace.
        let Err(PathValidationError::InvalidPath(message)) = resolve_within_root(&root, "dangling")
        else {
            panic!("a dangling leaf symlink must be refused as an invalid path");
        };
        let full_path = root.join("dangling");
        assert!(
            message.contains(full_path.to_str().unwrap()),
            "names the full path {full_path:?}: {message}"
        );
        assert!(
            message.contains("symlink target does not exist"),
            "explains the refusal: {message}"
        );
        assert!(!message.contains("os error"), "no raw io error: {message}");
    }

    #[cfg(unix)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_accepts_a_root_that_is_not_canonical() {
        // On macOS the temp dir sits behind the `/var` -> `/private/var` link.
        let tmp = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(tmp.path()).unwrap();
        std::fs::create_dir(canonical.join("src")).unwrap();
        let via_link = tmp.path().join("src").join("lib.rs");
        assert_eq!(
            resolve_within_root(tmp.path(), "src/lib.rs").unwrap(),
            canonical.join("src").join("lib.rs")
        );
        assert_eq!(
            resolve_within_root(tmp.path(), via_link.to_str().unwrap()).unwrap(),
            canonical.join("src").join("lib.rs")
        );
        assert!(matches!(
            resolve_within_root(tmp.path(), "../escape.txt"),
            Err(PathValidationError::OutsideRoot { .. })
        ));
    }

    #[spec("VAL-008")]
    #[test]
    fn resolve_through_existing_ancestors_never_falls_back_to_the_process_directory() {
        assert!(resolve_through_existing_ancestors(Path::new("relative/x.txt")).is_err());
    }

    #[cfg(windows)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_refuses_network_and_device_paths_before_touching_them() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        for hostile in [
            r"\\attacker\share\loot.txt",
            r"\\?\UNC\attacker\share\loot.txt",
            r"\\.\pipe\loot",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\loot",
        ] {
            assert!(
                matches!(
                    resolve_within_root(&root, hostile),
                    Err(PathValidationError::InvalidPath(_))
                ),
                "{hostile}"
            );
        }
    }

    #[cfg(windows)]
    #[spec("VAL-008")]
    #[test]
    fn resolve_within_root_refuses_a_different_drive_without_probing_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let root_drive =
            root.to_string_lossy().trim_start_matches(r"\\?\")[..1].to_ascii_uppercase();
        let other = if root_drive == "Z" { "Y" } else { "Z" };
        assert!(matches!(
            resolve_within_root(&root, &format!(r"{other}:\loot.txt")),
            Err(PathValidationError::InvalidPath(_))
        ));
        assert!(
            resolve_within_root(
                &root,
                &format!(r"{}:\ok.txt", root_drive.to_ascii_lowercase())
            )
            .is_ok()
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
