use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::anchor_client::{AnchorReceipt, AnchorRecord};
use crate::canonical_store::CanonicalStore;
use crate::merkle_tree::MerkleProof;

/// Complete provenance record for a single OpBundle.
#[derive(Debug, Clone)]
pub struct ProvenanceRecord {
    pub bundle_id: Uuid,
    pub originator: String,
    pub proposed_at: DateTime<Utc>,
    pub verified_at: Option<DateTime<Utc>>,
    pub applied_at: Option<DateTime<Utc>>,
    pub anchor: Option<AnchorReceipt>,
    pub merkle_proof: Option<MerkleProof>,
}

/// Query engine for end-to-end provenance traceability.
///
/// Links OpBundles through the canonical store and anchor records
/// to provide a complete audit trail from proposal to on-chain proof.
pub struct ProvenanceQuery {
    store: CanonicalStore,
    anchors: Vec<AnchorRecord>,
    records: Vec<ProvenanceRecord>,
}

impl ProvenanceQuery {
    /// Create a new provenance query engine.
    pub fn new(store: CanonicalStore, anchors: Vec<AnchorRecord>) -> Self {
        Self {
            store,
            anchors,
            records: Vec::new(),
        }
    }

    /// Register a provenance record for tracking.
    pub fn register(&mut self, record: ProvenanceRecord) {
        self.records.push(record);
    }

    /// Trace the provenance of a specific bundle by ID.
    pub fn trace_bundle(&self, bundle_id: Uuid) -> Option<&ProvenanceRecord> {
        self.records.iter().find(|r| r.bundle_id == bundle_id)
    }

    /// Trace all bundles that modified a given file.
    ///
    /// Walks the canonical store's ancestry chain and collects
    /// all bundle IDs that appear in version entries.
    pub fn trace_file(&self, _path: &str) -> Vec<Uuid> {
        let mut bundle_ids = Vec::new();
        if let Some(head_hash) = self.store.head_hash() {
            for hash in self.store.ancestry(&head_hash) {
                if let Some(entry) = self.store.get(&hash) {
                    bundle_ids.extend(entry.applied_bundles.iter().copied());
                }
            }
        }
        bundle_ids
    }

    /// Verify the full provenance chain for a record.
    ///
    /// Checks:
    /// 1. The bundle exists in the canonical store's ancestry
    /// 2. The Merkle proof is valid (if present)
    /// 3. The Merkle root matches an anchor record (if present)
    pub fn verify_provenance(&self, record: &ProvenanceRecord) -> bool {
        // Check bundle exists in store
        let in_store = self.store.head_hash().is_some_and(|head| {
            self.store.ancestry(&head).iter().any(|h| {
                self.store
                    .get(h)
                    .is_some_and(|e| e.applied_bundles.contains(&record.bundle_id))
            })
        });

        if !in_store {
            return false;
        }

        // If Merkle proof exists, verify it
        if let Some(proof) = &record.merkle_proof {
            if !proof.verify() {
                return false;
            }
            // Check root matches an anchor
            let root_anchored = self.anchors.iter().any(|a| a.merkle_root == proof.root);
            if !root_anchored {
                return false;
            }
        }

        true
    }

    /// Get all anchor records.
    pub fn anchors(&self) -> &[AnchorRecord] {
        &self.anchors
    }

    /// Get the canonical store.
    pub fn store(&self) -> &CanonicalStore {
        &self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(id: Uuid) -> ProvenanceRecord {
        ProvenanceRecord {
            bundle_id: id,
            originator: "agent-1".into(),
            proposed_at: Utc::now(),
            verified_at: Some(Utc::now()),
            applied_at: Some(Utc::now()),
            anchor: None,
            merkle_proof: None,
        }
    }

    #[test]
    fn trace_bundle_found() {
        let store = CanonicalStore::new();
        let mut pq = ProvenanceQuery::new(store, vec![]);
        let id = Uuid::new_v4();
        pq.register(make_record(id));
        assert!(pq.trace_bundle(id).is_some());
    }

    #[test]
    fn trace_bundle_not_found() {
        let store = CanonicalStore::new();
        let pq = ProvenanceQuery::new(store, vec![]);
        assert!(pq.trace_bundle(Uuid::new_v4()).is_none());
    }

    #[test]
    fn trace_file_follows_ancestry() {
        let mut store = CanonicalStore::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let h1 = store.store("v1", &[id1], None);
        let _h2 = store.store("v2", &[id2], Some(h1));

        let pq = ProvenanceQuery::new(store, vec![]);
        let ids = pq.trace_file("lib.rs");
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[test]
    fn verify_provenance_in_store() {
        let mut store = CanonicalStore::new();
        let id = Uuid::new_v4();
        store.store("source", &[id], None);

        let pq = ProvenanceQuery::new(store, vec![]);
        let record = make_record(id);
        assert!(pq.verify_provenance(&record));
    }

    #[test]
    fn verify_provenance_not_in_store() {
        let store = CanonicalStore::new();
        let pq = ProvenanceQuery::new(store, vec![]);
        let record = make_record(Uuid::new_v4());
        assert!(!pq.verify_provenance(&record));
    }

    #[test]
    fn verify_with_valid_merkle_proof() {
        use crate::merkle_tree::MerkleTree;
        use sha2::{Digest, Sha256};

        let mut store = CanonicalStore::new();
        let id = Uuid::new_v4();
        store.store("source", &[id], None);

        let leaf = {
            let mut h = Sha256::new();
            h.update(id.as_bytes());
            let r = h.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&r);
            out
        };

        let tree = MerkleTree::from_hashes(vec![leaf, [0u8; 32]]);
        let proof = tree.proof(0);

        let anchor = AnchorRecord {
            merkle_root: tree.root(),
            bundle_count: 1,
            timestamp: Utc::now(),
            signature: vec![],
        };

        let pq = ProvenanceQuery::new(store, vec![anchor]);
        let mut record = make_record(id);
        record.merkle_proof = Some(proof);
        assert!(pq.verify_provenance(&record));
    }

    #[test]
    fn verify_fails_with_unanchored_proof() {
        use crate::merkle_tree::MerkleTree;

        let mut store = CanonicalStore::new();
        let id = Uuid::new_v4();
        store.store("source", &[id], None);

        let tree = MerkleTree::from_hashes(vec![[1u8; 32], [2u8; 32]]);
        let proof = tree.proof(0);

        // No matching anchor
        let pq = ProvenanceQuery::new(store, vec![]);
        let mut record = make_record(id);
        record.merkle_proof = Some(proof);
        assert!(!pq.verify_provenance(&record));
    }

    #[test]
    fn empty_store_trace_file() {
        let store = CanonicalStore::new();
        let pq = ProvenanceQuery::new(store, vec![]);
        assert!(pq.trace_file("lib.rs").is_empty());
    }
}
