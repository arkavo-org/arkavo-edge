//! The directory a `shell_exec` command runs in.

use std::path::{Path, PathBuf};

/// Resolve the requested working directory inside `root`, or the root itself
/// when none is given. A relative request resolves against the root; one that
/// leaves it, directly or through a symlink, is refused rather than clamped.
/// The root is resolved too, so one that cannot be (an unreadable process
/// directory) denies the call instead of spawning in an unchecked place.
pub(super) fn confine(root: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    let dir = requested.unwrap_or(".");
    arkavo_validation::resolve_within_root(root, dir)
        .map_err(|e| format!("Working directory '{dir}' is outside the workspace root: {e}"))
}

/// The form of a confined directory that the shell can start in. Resolution
/// yields canonical paths, which on Windows are `\\?\C:\...` verbatim paths,
/// and `cmd.exe` rejects a verbatim or UNC directory as its working
/// directory. Only a verbatim drive path has a plain equivalent; any other
/// verbatim, UNC or device form is refused.
#[cfg(windows)]
pub(super) fn spawn_dir(dir: &Path) -> Result<PathBuf, String> {
    use std::path::{Component, Prefix};
    let refuse = || {
        format!(
            "Working directory '{}' cannot be used by cmd.exe",
            dir.display()
        )
    };
    let Some(Component::Prefix(prefix)) = dir.components().next() else {
        return Err(refuse());
    };
    match prefix.kind() {
        Prefix::Disk(_) => Ok(dir.to_path_buf()),
        Prefix::VerbatimDisk(_) => dir
            .to_str()
            .and_then(|s| s.strip_prefix(r"\\?\"))
            .map(PathBuf::from)
            .ok_or_else(refuse),
        _ => Err(refuse()),
    }
}

#[cfg(not(windows))]
pub(super) fn spawn_dir(dir: &Path) -> Result<PathBuf, String> {
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confine_defaults_to_root_and_refuses_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(confine(&root, None).unwrap(), root);
        std::fs::create_dir(root.join("sub")).unwrap();
        assert_eq!(confine(&root, Some("sub")).unwrap(), root.join("sub"));
        assert!(confine(&root, Some("..")).is_err());
        let outside = tempfile::tempdir().unwrap();
        let err = confine(&root, Some(outside.path().to_str().unwrap())).unwrap_err();
        assert!(err.contains("outside the workspace root"), "{err}");
        assert!(confine(Path::new(""), None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn spawn_dir_is_unchanged_off_windows() {
        assert_eq!(
            spawn_dir(Path::new("/tmp/x")).unwrap(),
            PathBuf::from("/tmp/x")
        );
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_drive_becomes_plain_and_other_verbatim_forms_are_refused() {
        assert_eq!(
            spawn_dir(Path::new(r"\\?\C:\work\sub")).unwrap(),
            PathBuf::from(r"C:\work\sub")
        );
        assert_eq!(
            spawn_dir(Path::new(r"C:\work")).unwrap(),
            PathBuf::from(r"C:\work")
        );
        assert!(spawn_dir(Path::new(r"\\?\UNC\host\share\dir")).is_err());
        assert!(spawn_dir(Path::new(r"\\host\share\dir")).is_err());
        assert!(spawn_dir(Path::new(r"\\?\Volume{0000}\dir")).is_err());
        assert!(spawn_dir(Path::new(r"\\.\C:\dir")).is_err());
        assert!(spawn_dir(Path::new(r"relative\dir")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_canonical_temp_directory_converts_to_a_plain_drive_path() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let plain = spawn_dir(&canonical).unwrap();
        assert!(!plain.to_string_lossy().starts_with(r"\\"), "{plain:?}");
        assert_eq!(std::fs::canonicalize(&plain).unwrap(), canonical);
    }
}
