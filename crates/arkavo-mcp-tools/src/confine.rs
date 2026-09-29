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
