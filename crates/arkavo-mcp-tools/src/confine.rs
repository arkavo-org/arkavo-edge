//! Workspace confinement shared by the tools that take a path from the model.
//!
//! Each tool's zero-config root is the process working directory, because an
//! agent process runs with its workspace as its working directory; embedders
//! that know better build the tool with `with_root`.

use std::path::{Path, PathBuf};

use crate::{Result, ToolError};

/// Resolve a model-supplied path inside `root`, or refuse the call. Every
/// file, TDF and git tool goes through this one mapping so an escape is always
/// a `PolicyDenied` and never a tool-specific error a caller might retry.
pub(crate) fn within_root(root: &Path, requested: &str) -> Result<PathBuf> {
    arkavo_validation::resolve_within_root(root, requested)
        .map_err(|e| ToolError::PolicyDenied(e.to_string()))
}

/// Require an already-existing `location` to live inside `root`, comparing
/// canonical forms so symlinked temp roots (macOS `/var`) still match. Used
/// where a library resolves a location itself (git repository discovery walks
/// up to an enclosing repository), so the path the caller passed proves nothing
/// about what was actually opened.
pub(crate) fn require_inside_root(root: &Path, location: &Path) -> Result<()> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| ToolError::PolicyDenied(format!("workspace root: {e}")))?;
    let canonical = std::fs::canonicalize(location)
        .map_err(|e| ToolError::PolicyDenied(format!("{}: {e}", location.display())))?;
    if canonical.starts_with(&canonical_root) {
        Ok(())
    } else {
        Err(ToolError::PolicyDenied(format!(
            "{} is outside allowed root {}",
            canonical.display(),
            canonical_root.display()
        )))
    }
}

/// Require every location an opened repository reads or writes to lie inside
/// `root`: its working directory (or git directory when bare), its git
/// directory, and its common directory. A workspace-controlled `.git` can name
/// an outer repository through `commondir` (refs and objects are then written
/// there) or through `objects/info/alternates` (its objects become readable),
/// so the working directory alone proves nothing. A legitimate linked worktree
/// passes only when its main repository is inside the root too.
pub(crate) fn require_repo_inside_root(root: &Path, repo: &git2::Repository) -> Result<()> {
    require_inside_root(root, repo.workdir().unwrap_or_else(|| repo.path()))?;
    require_inside_root(root, repo.path())?;
    let common = repo.commondir();
    require_inside_root(root, common)?;
    require_alternates_inside_root(root, &common.join("objects"))
}

fn require_alternates_inside_root(root: &Path, objects_dir: &Path) -> Result<()> {
    let listing = match std::fs::read_to_string(objects_dir.join("info").join("alternates")) {
        Ok(listing) => listing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(ToolError::PolicyDenied(format!(
                "objects/info/alternates: {e}"
            )));
        }
    };
    for line in listing.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Git also accepts C-style quoted paths; refusing them is safer than
        // re-implementing that unquoting inside a security check.
        if line.starts_with('"') {
            return Err(ToolError::PolicyDenied(
                "quoted entry in objects/info/alternates".to_string(),
            ));
        }
        // Relative entries resolve against the objects directory, as in git.
        require_inside_root(root, &objects_dir.join(line))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn escape_maps_to_policy_denied() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let err = within_root(ws.path(), outside.path().to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
    }

    #[test]
    fn require_inside_root_accepts_inside_and_refuses_outside_or_missing() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::create_dir(ws.path().join("sub")).unwrap();
        assert!(require_inside_root(ws.path(), &ws.path().join("sub")).is_ok());
        for bad in [outside.path().to_path_buf(), ws.path().join("missing")] {
            assert!(matches!(
                require_inside_root(ws.path(), &bad),
                Err(ToolError::PolicyDenied(_))
            ));
        }
        assert!(matches!(
            require_inside_root(&ws.path().join("missing"), ws.path()),
            Err(ToolError::PolicyDenied(_))
        ));
    }

    #[test]
    fn quoted_alternates_entry_is_refused() {
        let ws = TempDir::new().unwrap();
        let objects = ws.path().join("objects");
        std::fs::create_dir_all(objects.join("info")).unwrap();
        std::fs::write(objects.join("info/alternates"), "\"quoted\"\n").unwrap();
        assert!(matches!(
            require_alternates_inside_root(ws.path(), &objects),
            Err(ToolError::PolicyDenied(_))
        ));
    }

    #[test]
    fn empty_path_is_denied_and_inside_path_resolves() {
        let ws = TempDir::new().unwrap();
        assert!(matches!(
            within_root(ws.path(), ""),
            Err(ToolError::PolicyDenied(_))
        ));
        let root = std::fs::canonicalize(ws.path()).unwrap();
        assert_eq!(within_root(&root, "a/b.txt").unwrap(), root.join("a/b.txt"));
    }
}
