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
