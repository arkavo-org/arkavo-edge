//! Context manifest — registry mapping context names to `.bin` file metadata
//!
//! Tracks which pre-built context files exist, their hashes, and the model
//! they were built for. When the model changes, entries are flagged for
//! rebuild rather than silently serving stale KV cache data.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Metadata for a pre-built context file
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextEntry {
    pub name: String,
    pub bin_path: String,
    /// SHA-256 hex of the `.bin` file
    pub bin_hash: String,
    pub model_id: String,
    /// SHA-256 hex of the model — triggers rebuild when model changes
    pub model_hash: String,
    pub built_at: String,
    pub token_count: usize,
}

/// Registry mapping context names to `.bin` file metadata.
/// Local-first JSON format; iroh content-addressing layered on later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManifest {
    pub version: u32,
    pub entries: HashMap<String, ContextEntry>,
}

impl ContextManifest {
    pub fn new() -> Self {
        Self {
            version: 1,
            entries: HashMap::new(),
        }
    }

    /// Load a manifest from a JSON file.
    pub fn load(path: &str) -> Result<Self, ManifestError> {
        let data = std::fs::read_to_string(path)?;
        let manifest = serde_json::from_str(&data)?;
        Ok(manifest)
    }

    /// Save the manifest to a JSON file.
    pub fn save(&self, path: &str) -> Result<(), ManifestError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Look up an entry by name.
    pub fn get(&self, name: &str) -> Option<&ContextEntry> {
        self.entries.get(name)
    }

    /// Register or update an entry.
    pub fn register(&mut self, entry: ContextEntry) {
        self.entries.insert(entry.name.clone(), entry);
    }

    /// Remove an entry by name. Returns the removed entry if present.
    pub fn remove(&mut self, name: &str) -> Option<ContextEntry> {
        self.entries.remove(name)
    }

    /// Check if an entry needs rebuilding because the model has changed.
    pub fn needs_rebuild(&self, name: &str, current_model_hash: &str) -> bool {
        self.entries
            .get(name)
            .is_none_or(|e| e.model_hash != current_model_hash)
    }

    /// Remove entries whose `.bin` file no longer exists on disk.
    /// Returns the number of entries pruned.
    pub fn prune(&mut self) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|_, entry| Path::new(&entry.bin_path).exists());
        before - self.entries.len()
    }

    /// Number of registered entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ContextManifest {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(name: &str) -> ContextEntry {
        ContextEntry {
            name: name.to_string(),
            bin_path: format!("/tmp/ctx/{name}.bin"),
            bin_hash: "abc123".to_string(),
            model_id: "qwen3.5-0.8b".to_string(),
            model_hash: "model_hash_v1".to_string(),
            built_at: "2026-02-22T00:00:00Z".to_string(),
            token_count: 512,
        }
    }

    #[test]
    fn test_manifest_new() {
        let m = ContextManifest::new();
        assert_eq!(m.version, 1);
        assert!(m.is_empty());
    }

    #[test]
    fn test_register_and_get() {
        let mut m = ContextManifest::new();
        m.register(sample_entry("role"));
        assert_eq!(m.len(), 1);
        let entry = m.get("role").unwrap();
        assert_eq!(entry.name, "role");
        assert_eq!(entry.token_count, 512);
    }

    #[test]
    fn test_remove() {
        let mut m = ContextManifest::new();
        m.register(sample_entry("role"));
        let removed = m.remove("role");
        assert!(removed.is_some());
        assert!(m.is_empty());
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut m = ContextManifest::new();
        assert!(m.remove("nope").is_none());
    }

    #[test]
    fn test_needs_rebuild_missing_entry() {
        let m = ContextManifest::new();
        assert!(m.needs_rebuild("role", "any_hash"));
    }

    #[test]
    fn test_needs_rebuild_same_model() {
        let mut m = ContextManifest::new();
        m.register(sample_entry("role"));
        assert!(!m.needs_rebuild("role", "model_hash_v1"));
    }

    #[test]
    fn test_needs_rebuild_different_model() {
        let mut m = ContextManifest::new();
        m.register(sample_entry("role"));
        assert!(m.needs_rebuild("role", "model_hash_v2"));
    }

    #[test]
    fn test_prune_removes_missing_files() {
        let mut m = ContextManifest::new();
        m.register(sample_entry("ghost"));
        let pruned = m.prune();
        assert_eq!(pruned, 1);
        assert!(m.is_empty());
    }

    #[test]
    fn test_prune_keeps_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("kept.bin");
        std::fs::write(&bin_path, b"data").unwrap();

        let mut m = ContextManifest::new();
        let mut entry = sample_entry("kept");
        entry.bin_path = bin_path.to_str().unwrap().to_string();
        m.register(entry);

        let pruned = m.prune();
        assert_eq!(pruned, 0);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn test_register_overwrites() {
        let mut m = ContextManifest::new();
        m.register(sample_entry("role"));
        let mut updated = sample_entry("role");
        updated.token_count = 1024;
        m.register(updated);
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("role").unwrap().token_count, 1024);
    }

    #[test]
    fn test_manifest_serialization() {
        let mut m = ContextManifest::new();
        m.register(sample_entry("role"));
        m.register(sample_entry("policy"));

        let json = serde_json::to_string(&m).unwrap();
        let deserialized: ContextManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.version, 1);
        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized.get("role").unwrap(), m.get("role").unwrap());
    }

    #[test]
    fn test_manifest_default() {
        let m = ContextManifest::default();
        assert_eq!(m.version, 1);
        assert!(m.is_empty());
    }

    #[test]
    fn test_context_entry_clone() {
        let entry = sample_entry("test");
        let cloned = entry.clone();
        assert_eq!(entry, cloned);
    }
}
