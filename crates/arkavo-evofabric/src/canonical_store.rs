use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

/// Content-addressed storage for canonical AST versions.
///
/// Each entry is identified by the SHA-256 of its rendered source (via prettyplease),
/// providing deterministic, deduplication-friendly storage for the version DAG.
#[derive(Debug, Default)]
pub struct CanonicalStore {
    versions: HashMap<[u8; 32], CanonicalEntry>,
    head: Option<[u8; 32]>,
}

/// A single version in the content-addressed store.
#[derive(Debug, Clone)]
pub struct CanonicalEntry {
    pub source_hash: [u8; 32],
    pub rendered_source: String,
    pub parent: Option<[u8; 32]>,
    pub applied_bundles: Vec<Uuid>,
    pub timestamp: DateTime<Utc>,
}

impl CanonicalStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a new version, returning its content-address hash.
    ///
    /// The hash is computed from the rendered source string. If the same source
    /// is stored again, it returns the existing hash without duplicating.
    pub fn store(
        &mut self,
        rendered_source: &str,
        bundles: &[Uuid],
        parent: Option<[u8; 32]>,
    ) -> [u8; 32] {
        let hash = content_hash(rendered_source);

        self.versions.entry(hash).or_insert_with(|| CanonicalEntry {
            source_hash: hash,
            rendered_source: rendered_source.to_string(),
            parent,
            applied_bundles: bundles.to_vec(),
            timestamp: Utc::now(),
        });

        self.head = Some(hash);
        hash
    }

    /// Retrieve a version by its content hash.
    pub fn get(&self, hash: &[u8; 32]) -> Option<&CanonicalEntry> {
        self.versions.get(hash)
    }

    /// Get the current head version.
    pub fn head(&self) -> Option<&CanonicalEntry> {
        self.head.and_then(|h| self.versions.get(&h))
    }

    /// Get the head hash.
    pub fn head_hash(&self) -> Option<[u8; 32]> {
        self.head
    }

    /// Walk the ancestry chain from a given hash back to the root.
    pub fn ancestry(&self, hash: &[u8; 32]) -> Vec<[u8; 32]> {
        let mut chain = Vec::new();
        let mut current = Some(*hash);
        while let Some(h) = current {
            if chain.contains(&h) {
                break; // safety: prevent cycles
            }
            chain.push(h);
            current = self.versions.get(&h).and_then(|e| e.parent);
        }
        chain
    }

    /// Number of stored versions.
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    /// Check if a version exists.
    pub fn contains(&self, hash: &[u8; 32]) -> bool {
        self.versions.contains_key(hash)
    }
}

/// Compute the SHA-256 content hash of rendered source.
pub fn content_hash(source: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("EVO-004")]
    #[test]
    fn empty_store() {
        let store = CanonicalStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        assert!(store.head().is_none());
    }

    #[spec("EVO-004")]
    #[test]
    fn store_and_retrieve() {
        let mut store = CanonicalStore::new();
        let hash = store.store("fn main() {}", &[], None);
        assert_eq!(store.len(), 1);
        let entry = store.get(&hash).unwrap();
        assert_eq!(entry.rendered_source, "fn main() {}");
        assert!(entry.parent.is_none());
    }

    #[spec("EVO-004")]
    #[test]
    fn head_tracks_latest() {
        let mut store = CanonicalStore::new();
        let h1 = store.store("fn a() {}", &[], None);
        assert_eq!(store.head_hash(), Some(h1));
        let h2 = store.store("fn b() {}", &[], Some(h1));
        assert_eq!(store.head_hash(), Some(h2));
    }

    #[spec("EVO-004")]
    #[test]
    fn deduplicates_same_source() {
        let mut store = CanonicalStore::new();
        let h1 = store.store("fn main() {}", &[], None);
        let h2 = store.store("fn main() {}", &[], None);
        assert_eq!(h1, h2);
        assert_eq!(store.len(), 1);
    }

    #[spec("EVO-004")]
    #[test]
    fn ancestry_chain() {
        let mut store = CanonicalStore::new();
        let h1 = store.store("v1", &[], None);
        let h2 = store.store("v2", &[], Some(h1));
        let h3 = store.store("v3", &[], Some(h2));
        let chain = store.ancestry(&h3);
        assert_eq!(chain, vec![h3, h2, h1]);
    }

    #[spec("EVO-004")]
    #[test]
    fn ancestry_single_version() {
        let mut store = CanonicalStore::new();
        let h = store.store("only", &[], None);
        assert_eq!(store.ancestry(&h), vec![h]);
    }

    #[spec("EVO-004")]
    #[test]
    fn content_hash_deterministic() {
        let h1 = content_hash("fn foo() {}");
        let h2 = content_hash("fn foo() {}");
        assert_eq!(h1, h2);
    }

    #[spec("EVO-004")]
    #[test]
    fn content_hash_differs_for_different_source() {
        let h1 = content_hash("fn foo() {}");
        let h2 = content_hash("fn bar() {}");
        assert_ne!(h1, h2);
    }

    #[spec("EVO-004")]
    #[test]
    fn bundles_stored() {
        let mut store = CanonicalStore::new();
        let id = Uuid::new_v4();
        let hash = store.store("fn x() {}", &[id], None);
        let entry = store.get(&hash).unwrap();
        assert_eq!(entry.applied_bundles, vec![id]);
    }

    #[spec("EVO-004")]
    #[test]
    fn contains_check() {
        let mut store = CanonicalStore::new();
        let h = store.store("src", &[], None);
        assert!(store.contains(&h));
        assert!(!store.contains(&[0xFF; 32]));
    }
}
