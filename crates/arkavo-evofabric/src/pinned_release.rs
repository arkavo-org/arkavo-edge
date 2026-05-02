use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// A named pointer to a specific canonical version for instant rollback.
#[derive(Debug, Clone)]
pub struct PinnedRelease {
    pub name: String,
    pub version_hash: [u8; 32],
    pub pinned_at: DateTime<Utc>,
    pub pinned_by: String,
}

/// Registry of named release pins.
#[derive(Debug, Default)]
pub struct ReleaseRegistry {
    releases: HashMap<String, PinnedRelease>,
}

impl ReleaseRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin a name to a version hash. Overwrites any existing pin with the same name.
    pub fn pin(&mut self, name: &str, hash: [u8; 32], agent: &str) {
        self.releases.insert(
            name.to_string(),
            PinnedRelease {
                name: name.to_string(),
                version_hash: hash,
                pinned_at: Utc::now(),
                pinned_by: agent.to_string(),
            },
        );
    }

    /// Get the version hash for a named release.
    pub fn rollback_to(&self, name: &str) -> Option<[u8; 32]> {
        self.releases.get(name).map(|r| r.version_hash)
    }

    /// List all pinned releases.
    pub fn list(&self) -> Vec<&PinnedRelease> {
        let mut releases: Vec<_> = self.releases.values().collect();
        releases.sort_by(|a, b| a.name.cmp(&b.name));
        releases
    }

    /// Remove a pinned release by name.
    pub fn unpin(&mut self, name: &str) -> bool {
        self.releases.remove(name).is_some()
    }

    /// Number of pinned releases.
    pub fn len(&self) -> usize {
        self.releases.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.releases.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("EVO-006")]
    #[test]
    fn empty_registry() {
        let reg = ReleaseRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.list().is_empty());
    }

    #[spec("EVO-006")]
    #[test]
    fn pin_and_rollback() {
        let mut reg = ReleaseRegistry::new();
        let hash = [42u8; 32];
        reg.pin("v1.0", hash, "agent-1");
        assert_eq!(reg.rollback_to("v1.0"), Some(hash));
    }

    #[spec("EVO-006")]
    #[test]
    fn rollback_missing_returns_none() {
        let reg = ReleaseRegistry::new();
        assert_eq!(reg.rollback_to("v1.0"), None);
    }

    #[spec("EVO-006")]
    #[test]
    fn pin_overwrites() {
        let mut reg = ReleaseRegistry::new();
        reg.pin("stable", [1u8; 32], "a1");
        reg.pin("stable", [2u8; 32], "a2");
        assert_eq!(reg.rollback_to("stable"), Some([2u8; 32]));
        assert_eq!(reg.len(), 1);
    }

    #[spec("EVO-006")]
    #[test]
    fn list_sorted_by_name() {
        let mut reg = ReleaseRegistry::new();
        reg.pin("beta", [2u8; 32], "a");
        reg.pin("alpha", [1u8; 32], "a");
        reg.pin("stable", [3u8; 32], "a");
        let names: Vec<_> = reg.list().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "stable"]);
    }

    #[spec("EVO-006")]
    #[test]
    fn unpin_removes() {
        let mut reg = ReleaseRegistry::new();
        reg.pin("v1.0", [1u8; 32], "a");
        assert!(reg.unpin("v1.0"));
        assert!(reg.is_empty());
        assert!(!reg.unpin("v1.0")); // already removed
    }
}
