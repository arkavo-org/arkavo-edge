use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A single version in the canonical version DAG.
#[derive(Debug, Clone)]
pub struct CanonicalVersion {
    pub id: [u8; 32],
    pub source_hash: [u8; 32],
    pub parent: Option<[u8; 32]>,
    pub applied_bundles: Vec<Uuid>,
    pub timestamp: DateTime<Utc>,
}

/// DAG of canonical versions, supporting ancestry traversal and common ancestor queries.
#[derive(Debug, Default)]
pub struct VersionDag {
    versions: HashMap<[u8; 32], CanonicalVersion>,
}

impl VersionDag {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a version to the DAG.
    pub fn add(&mut self, version: CanonicalVersion) {
        self.versions.insert(version.id, version);
    }

    /// Get a version by ID.
    pub fn get(&self, id: &[u8; 32]) -> Option<&CanonicalVersion> {
        self.versions.get(id)
    }

    /// Walk ancestors from a given version back to the root.
    pub fn ancestors(&self, id: &[u8; 32]) -> Vec<[u8; 32]> {
        let mut chain = Vec::new();
        let mut current = Some(*id);
        let mut visited = HashSet::new();
        while let Some(h) = current {
            if !visited.insert(h) {
                break;
            }
            chain.push(h);
            current = self.versions.get(&h).and_then(|v| v.parent);
        }
        chain
    }

    /// Find the most recent common ancestor of two versions.
    pub fn common_ancestor(&self, a: &[u8; 32], b: &[u8; 32]) -> Option<[u8; 32]> {
        let ancestors_a: HashSet<_> = self.ancestors(a).into_iter().collect();
        let chain_b = self.ancestors(b);
        chain_b.into_iter().find(|h| ancestors_a.contains(h))
    }

    /// Number of versions in the DAG.
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Whether the DAG is empty.
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn version(id: u8, parent: Option<u8>) -> CanonicalVersion {
        CanonicalVersion {
            id: [id; 32],
            source_hash: [id; 32],
            parent: parent.map(|p| [p; 32]),
            applied_bundles: vec![],
            timestamp: Utc::now(),
        }
    }

    #[spec("EVO-004")]
    #[test]
    fn empty_dag() {
        let dag = VersionDag::new();
        assert!(dag.is_empty());
        assert_eq!(dag.len(), 0);
    }

    #[spec("EVO-004")]
    #[test]
    fn add_and_get() {
        let mut dag = VersionDag::new();
        dag.add(version(1, None));
        assert!(dag.get(&[1; 32]).is_some());
        assert!(dag.get(&[2; 32]).is_none());
    }

    #[spec("EVO-004")]
    #[test]
    fn linear_ancestry() {
        let mut dag = VersionDag::new();
        dag.add(version(1, None));
        dag.add(version(2, Some(1)));
        dag.add(version(3, Some(2)));
        let chain = dag.ancestors(&[3; 32]);
        assert_eq!(chain, vec![[3; 32], [2; 32], [1; 32]]);
    }

    #[spec("EVO-004")]
    #[test]
    fn common_ancestor_linear() {
        let mut dag = VersionDag::new();
        dag.add(version(1, None));
        dag.add(version(2, Some(1)));
        dag.add(version(3, Some(2)));
        assert_eq!(dag.common_ancestor(&[3; 32], &[2; 32]), Some([2; 32]));
    }

    #[spec("EVO-004")]
    #[test]
    fn common_ancestor_fork() {
        let mut dag = VersionDag::new();
        dag.add(version(1, None));
        dag.add(version(2, Some(1)));
        dag.add(version(3, Some(1))); // fork from 1
        assert_eq!(dag.common_ancestor(&[2; 32], &[3; 32]), Some([1; 32]));
    }

    #[spec("EVO-004")]
    #[test]
    fn common_ancestor_none() {
        let mut dag = VersionDag::new();
        dag.add(version(1, None));
        dag.add(version(2, None)); // independent root
        assert_eq!(dag.common_ancestor(&[1; 32], &[2; 32]), None);
    }

    #[spec("EVO-004")]
    #[test]
    fn ancestors_missing_version() {
        let dag = VersionDag::new();
        let chain = dag.ancestors(&[99; 32]);
        assert_eq!(chain, vec![[99; 32]]);
    }

    #[spec("EVO-004")]
    #[test]
    fn common_ancestor_same_version() {
        let mut dag = VersionDag::new();
        dag.add(version(1, None));
        assert_eq!(dag.common_ancestor(&[1; 32], &[1; 32]), Some([1; 32]));
    }
}
