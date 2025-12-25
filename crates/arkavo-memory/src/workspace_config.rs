//! Workspace configuration discovery and caching.
//!
//! Discovers workspace paths from .arkavo/AGENTS.md and caches them
//! for consistent cross-process database access.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Cached workspace configuration for cross-process consistency.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceConfig {
    /// Absolute path to the memory database
    pub memory_db_path: Option<PathBuf>,
    /// Workspace root directory (parent of .arkavo)
    pub workspace_root: Option<PathBuf>,
}

static WORKSPACE_CONFIG: OnceLock<WorkspaceConfig> = OnceLock::new();

impl WorkspaceConfig {
    /// Get the cached workspace configuration.
    /// Discovers and caches on first call.
    pub fn get() -> &'static WorkspaceConfig {
        WORKSPACE_CONFIG.get_or_init(|| Self::discover().unwrap_or_default())
    }

    /// Discover workspace configuration by walking up directory tree.
    fn discover() -> Option<WorkspaceConfig> {
        let cwd = std::env::current_dir().ok()?;
        Self::find_from_path(&cwd)
    }

    /// Find workspace configuration starting from a given path.
    /// Walks up the directory tree looking for .arkavo/AGENTS.md.
    pub fn find_from_path(start: &Path) -> Option<WorkspaceConfig> {
        let mut current = start.to_path_buf();

        loop {
            let agents_md_path = current.join(".arkavo").join("AGENTS.md");

            if agents_md_path.exists() {
                // Found AGENTS.md, parse it
                if let Ok(content) = std::fs::read_to_string(&agents_md_path) {
                    let memory_db_path = parse_memory_db_path(&content, &current);
                    return Some(WorkspaceConfig {
                        memory_db_path,
                        workspace_root: Some(current),
                    });
                }
            }

            // Check if .arkavo directory exists without AGENTS.md
            let arkavo_dir = current.join(".arkavo");
            if arkavo_dir.is_dir() {
                // Bootstrap with default paths using this workspace root
                let default_db = current
                    .join(".arkavo")
                    .join("memory_server")
                    .join("memories.db");
                return Some(WorkspaceConfig {
                    memory_db_path: Some(default_db),
                    workspace_root: Some(current),
                });
            }

            // Move to parent directory
            if !current.pop() {
                break;
            }
        }

        None
    }

    /// Get the memory database path, falling back to default if not configured.
    pub fn memory_db_path_or_default(&self) -> PathBuf {
        self.memory_db_path.clone().unwrap_or_else(|| {
            PathBuf::from(".arkavo")
                .join("memory_server")
                .join("memories.db")
        })
    }
}

/// Parse memory_db path from AGENTS.md content.
/// Resolves relative paths against the workspace root.
fn parse_memory_db_path(content: &str, workspace_root: &Path) -> Option<PathBuf> {
    let mut in_paths_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for Paths section header
        if trimmed == "## Paths" {
            in_paths_section = true;
            continue;
        }

        // End paths section on next ## header
        if in_paths_section && trimmed.starts_with("## ") {
            break;
        }

        // Parse paths: section marker
        if trimmed == "paths:" {
            in_paths_section = true;
            continue;
        }

        // Parse memory_db path
        if in_paths_section && trimmed.starts_with("memory_db:") {
            let path_str = trimmed
                .strip_prefix("memory_db:")
                .unwrap_or("")
                .trim()
                .trim_matches('"');

            if !path_str.is_empty() {
                let path = PathBuf::from(path_str);
                // Resolve relative paths against workspace root
                let resolved = if path.is_absolute() {
                    path
                } else {
                    workspace_root.join(path)
                };
                return Some(resolved);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_memory_db_path_relative() {
        let content = r#"
## Paths

paths:
  memory_db: .arkavo/memory_server/memories.db
"#;
        let workspace_root = PathBuf::from("/home/user/project");
        let path = parse_memory_db_path(content, &workspace_root);

        assert_eq!(
            path,
            Some(PathBuf::from(
                "/home/user/project/.arkavo/memory_server/memories.db"
            ))
        );
    }

    #[test]
    fn test_parse_memory_db_path_absolute() {
        let content = r#"
## Paths

paths:
  memory_db: /absolute/path/to/memories.db
"#;
        let workspace_root = PathBuf::from("/home/user/project");
        let path = parse_memory_db_path(content, &workspace_root);

        assert_eq!(path, Some(PathBuf::from("/absolute/path/to/memories.db")));
    }

    #[test]
    fn test_parse_memory_db_path_no_paths_section() {
        let content = r#"
## Agent

name: test-agent
model: gpt-4
"#;
        let workspace_root = PathBuf::from("/home/user/project");
        let path = parse_memory_db_path(content, &workspace_root);

        assert_eq!(path, None);
    }

    #[test]
    fn test_find_from_path_with_agents_md() {
        let temp_dir = tempfile::tempdir().unwrap();
        let arkavo_dir = temp_dir.path().join(".arkavo");
        fs::create_dir_all(&arkavo_dir).unwrap();

        let agents_md = arkavo_dir.join("AGENTS.md");
        fs::write(
            &agents_md,
            r#"
## Paths

paths:
  memory_db: .arkavo/memory_server/memories.db
"#,
        )
        .unwrap();

        let config = WorkspaceConfig::find_from_path(temp_dir.path()).unwrap();

        assert_eq!(config.workspace_root, Some(temp_dir.path().to_path_buf()));
        assert_eq!(
            config.memory_db_path,
            Some(temp_dir.path().join(".arkavo/memory_server/memories.db"))
        );
    }

    #[test]
    fn test_find_from_path_with_arkavo_dir_only() {
        let temp_dir = tempfile::tempdir().unwrap();
        let arkavo_dir = temp_dir.path().join(".arkavo");
        fs::create_dir_all(&arkavo_dir).unwrap();

        let config = WorkspaceConfig::find_from_path(temp_dir.path()).unwrap();

        assert_eq!(config.workspace_root, Some(temp_dir.path().to_path_buf()));
        assert_eq!(
            config.memory_db_path,
            Some(temp_dir.path().join(".arkavo/memory_server/memories.db"))
        );
    }

    #[test]
    fn test_find_from_path_walks_up() {
        let temp_dir = tempfile::tempdir().unwrap();
        let arkavo_dir = temp_dir.path().join(".arkavo");
        fs::create_dir_all(&arkavo_dir).unwrap();

        // Create nested subdirectory
        let nested = temp_dir.path().join("src").join("lib");
        fs::create_dir_all(&nested).unwrap();

        let config = WorkspaceConfig::find_from_path(&nested).unwrap();

        assert_eq!(config.workspace_root, Some(temp_dir.path().to_path_buf()));
    }

    #[test]
    fn test_find_from_path_no_arkavo() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig::find_from_path(temp_dir.path());

        assert!(config.is_none());
    }

    #[test]
    fn test_memory_db_path_or_default() {
        let config = WorkspaceConfig::default();
        let default_path = config.memory_db_path_or_default();

        assert_eq!(
            default_path,
            PathBuf::from(".arkavo/memory_server/memories.db")
        );
    }
}
