//! Workspace configuration discovery and caching.
//!
//! Discovers the workspace root by walking up from a starting path looking
//! for a `.arkavo/` directory, and caches it for consistent cross-process
//! database access. The memory database path is always the default under
//! `.arkavo/memory_server/memories.db` relative to that root — the old
//! AGENTS.md `## Paths` / `memory_db:` override has been removed, and no
//! replacement override exists in the SwarmKit manifest schema
//! (`KitRuntimeConfig` has no `memory_db` field).

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
    /// Walks up the directory tree looking for a `.arkavo/` directory.
    pub fn find_from_path(start: &Path) -> Option<WorkspaceConfig> {
        let mut current = start.to_path_buf();

        loop {
            let arkavo_dir = current.join(".arkavo");
            if arkavo_dir.is_dir() {
                // Bootstrap with default paths using this workspace root
                let default_db = arkavo_dir.join("memory_server").join("memories.db");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_from_path_with_arkavo_dir() {
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
