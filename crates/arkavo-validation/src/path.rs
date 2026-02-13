use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_relative_path() {
        let root = Path::new("/workspace");
        let result = validate_path_within_root(root, "src/main.rs");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/workspace/src/main.rs"));
    }

    #[test]
    fn test_traversal_blocked() {
        let root = Path::new("/workspace");
        assert!(validate_path_within_root(root, "../etc/passwd").is_err());
        assert!(validate_path_within_root(root, "src/../../etc").is_err());
    }

    #[test]
    fn test_tilde_blocked() {
        let root = Path::new("/workspace");
        assert!(validate_path_within_root(root, "~/secret").is_err());
    }

    #[test]
    fn test_absolute_outside_root() {
        let root = Path::new("/workspace");
        assert!(validate_path_within_root(root, "/etc/passwd").is_err());
    }

    #[test]
    fn test_absolute_inside_root() {
        let root = Path::new("/workspace");
        let result = validate_path_within_root(root, "/workspace/src/lib.rs");
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_traversal_allows_absolute() {
        let base = Path::new("/workspace");
        let result = validate_no_traversal(base, "/tmp/file.txt");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/tmp/file.txt"));
    }

    #[test]
    fn test_no_traversal_blocks_dotdot() {
        let base = Path::new("/workspace");
        assert!(validate_no_traversal(base, "../etc/passwd").is_err());
    }
}
