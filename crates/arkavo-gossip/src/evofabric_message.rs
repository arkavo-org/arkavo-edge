//! Gossip message types for EvoFabric code evolution coordination
//!
//! Enables agents to propose, verify, merge, and anchor OpBundle changes
//! across the swarm. Follows the same signed-message pattern as
//! ExperimentAnnouncement.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Proposal to apply an OpBundle to the canonical AST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvoFabricProposal {
    pub bundle_id: Uuid,
    pub bundle_hash: [u8; 32],
    pub originator: String,
    pub target_file: String,
    pub clock: BTreeMap<String, u64>,
    pub timestamp: DateTime<Utc>,
    pub signature: Vec<u8>,
}

impl EvoFabricProposal {
    #[must_use]
    pub fn new(
        bundle_id: Uuid,
        bundle_hash: [u8; 32],
        originator: String,
        target_file: String,
        clock: BTreeMap<String, u64>,
    ) -> Self {
        Self {
            bundle_id,
            bundle_hash,
            originator,
            target_file,
            clock,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.bundle_id.as_bytes());
        content.extend(&self.bundle_hash);
        content.extend(self.originator.as_bytes());
        content.extend(self.target_file.as_bytes());
        for (k, v) in &self.clock {
            content.extend(k.as_bytes());
            content.extend(v.to_le_bytes());
        }
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

/// Verification result for an OpBundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvoFabricVerification {
    pub bundle_id: Uuid,
    pub verifier: String,
    pub verified: bool,
    pub compile_ok: bool,
    pub test_ok: bool,
    pub timestamp: DateTime<Utc>,
    pub signature: Vec<u8>,
}

impl EvoFabricVerification {
    #[must_use]
    pub fn new(
        bundle_id: Uuid,
        verifier: String,
        verified: bool,
        compile_ok: bool,
        test_ok: bool,
    ) -> Self {
        Self {
            bundle_id,
            verifier,
            verified,
            compile_ok,
            test_ok,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.bundle_id.as_bytes());
        content.extend(self.verifier.as_bytes());
        content.push(if self.verified { 1 } else { 0 });
        content.push(if self.compile_ok { 1 } else { 0 });
        content.push(if self.test_ok { 1 } else { 0 });
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

/// Decision on whether to merge an OpBundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeDecisionKind {
    Accept,
    Reject,
    Defer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvoFabricMergeDecision {
    pub bundle_id: Uuid,
    pub decision: MergeDecisionKind,
    pub decider: String,
    pub timestamp: DateTime<Utc>,
    pub signature: Vec<u8>,
}

impl EvoFabricMergeDecision {
    #[must_use]
    pub fn new(bundle_id: Uuid, decision: MergeDecisionKind, decider: String) -> Self {
        Self {
            bundle_id,
            decision,
            decider,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.bundle_id.as_bytes());
        content.push(match self.decision {
            MergeDecisionKind::Accept => 1,
            MergeDecisionKind::Reject => 2,
            MergeDecisionKind::Defer => 3,
        });
        content.extend(self.decider.as_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

/// Notice that two bundles are in conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvoFabricConflictNotice {
    pub bundle_a: Uuid,
    pub bundle_b: Uuid,
    pub conflict_kind: String,
    pub reporter: String,
    pub timestamp: DateTime<Utc>,
    pub signature: Vec<u8>,
}

impl EvoFabricConflictNotice {
    #[must_use]
    pub fn new(bundle_a: Uuid, bundle_b: Uuid, conflict_kind: String, reporter: String) -> Self {
        Self {
            bundle_a,
            bundle_b,
            conflict_kind,
            reporter,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.bundle_a.as_bytes());
        content.extend(self.bundle_b.as_bytes());
        content.extend(self.conflict_kind.as_bytes());
        content.extend(self.reporter.as_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

/// Announcement that a Merkle root has been anchored on-chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvoFabricAnchorCommitted {
    pub merkle_root: [u8; 32],
    pub block_number: u64,
    pub bundle_count: usize,
    pub timestamp: DateTime<Utc>,
    pub signature: Vec<u8>,
}

impl EvoFabricAnchorCommitted {
    #[must_use]
    pub fn new(merkle_root: [u8; 32], block_number: u64, bundle_count: usize) -> Self {
        Self {
            merkle_root,
            block_number,
            bundle_count,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(&self.merkle_root);
        content.extend(self.block_number.to_le_bytes());
        content.extend(self.bundle_count.to_le_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_content_to_sign_deterministic() {
        let p = EvoFabricProposal::new(
            Uuid::nil(),
            [1u8; 32],
            "agent-1".into(),
            "lib.rs".into(),
            BTreeMap::new(),
        );
        assert_eq!(p.content_to_sign(), p.content_to_sign());
    }

    #[test]
    fn proposal_content_differs_by_file() {
        let a = EvoFabricProposal::new(
            Uuid::nil(),
            [1u8; 32],
            "agent-1".into(),
            "a.rs".into(),
            BTreeMap::new(),
        );
        let b = EvoFabricProposal::new(
            Uuid::nil(),
            [1u8; 32],
            "agent-1".into(),
            "b.rs".into(),
            BTreeMap::new(),
        );
        assert_ne!(a.content_to_sign(), b.content_to_sign());
    }

    #[test]
    fn verification_content_to_sign() {
        let v = EvoFabricVerification::new(Uuid::nil(), "verifier".into(), true, true, true);
        assert!(!v.content_to_sign().is_empty());
    }

    #[test]
    fn verification_differs_by_result() {
        let pass = EvoFabricVerification::new(Uuid::nil(), "v".into(), true, true, true);
        let mut fail = pass.clone();
        fail.verified = false;
        assert_ne!(pass.content_to_sign(), fail.content_to_sign());
    }

    #[test]
    fn merge_decision_content_to_sign() {
        let d = EvoFabricMergeDecision::new(Uuid::nil(), MergeDecisionKind::Accept, "d".into());
        assert!(!d.content_to_sign().is_empty());
    }

    #[test]
    fn merge_decision_differs_by_kind() {
        let accept =
            EvoFabricMergeDecision::new(Uuid::nil(), MergeDecisionKind::Accept, "d".into());
        let mut reject = accept.clone();
        reject.decision = MergeDecisionKind::Reject;
        assert_ne!(accept.content_to_sign(), reject.content_to_sign());
    }

    #[test]
    fn conflict_notice_content_to_sign() {
        let c = EvoFabricConflictNotice::new(
            Uuid::nil(),
            Uuid::nil(),
            "overlapping".into(),
            "r".into(),
        );
        assert!(!c.content_to_sign().is_empty());
    }

    #[test]
    fn conflict_notice_differs_by_bundles() {
        let a = EvoFabricConflictNotice::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "overlapping".into(),
            "r".into(),
        );
        let b = EvoFabricConflictNotice::new(
            Uuid::from_u128(3),
            Uuid::from_u128(4),
            "overlapping".into(),
            "r".into(),
        );
        assert_ne!(a.content_to_sign(), b.content_to_sign());
    }

    #[test]
    fn anchor_committed_content_to_sign() {
        let a = EvoFabricAnchorCommitted::new([0u8; 32], 100, 5);
        assert!(!a.content_to_sign().is_empty());
    }

    #[test]
    fn anchor_committed_differs_by_block() {
        let a = EvoFabricAnchorCommitted::new([0u8; 32], 100, 5);
        let mut b = a.clone();
        b.block_number = 200;
        assert_ne!(a.content_to_sign(), b.content_to_sign());
    }

    #[test]
    fn proposal_serde_roundtrip() {
        let p = EvoFabricProposal::new(
            Uuid::new_v4(),
            [42u8; 32],
            "agent-1".into(),
            "lib.rs".into(),
            BTreeMap::from([("a".into(), 1), ("b".into(), 2)]),
        );
        let json = serde_json::to_string(&p).unwrap();
        let restored: EvoFabricProposal = serde_json::from_str(&json).unwrap();
        assert_eq!(p.bundle_id, restored.bundle_id);
        assert_eq!(p.clock, restored.clock);
    }

    #[test]
    fn verification_serde_roundtrip() {
        let v = EvoFabricVerification::new(Uuid::new_v4(), "v1".into(), true, true, false);
        let json = serde_json::to_string(&v).unwrap();
        let restored: EvoFabricVerification = serde_json::from_str(&json).unwrap();
        assert_eq!(v.bundle_id, restored.bundle_id);
        assert_eq!(v.test_ok, restored.test_ok);
    }

    #[test]
    fn merge_decision_serde_roundtrip() {
        let d = EvoFabricMergeDecision::new(Uuid::new_v4(), MergeDecisionKind::Defer, "d1".into());
        let json = serde_json::to_string(&d).unwrap();
        let restored: EvoFabricMergeDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(d.decision, restored.decision);
    }

    #[test]
    fn anchor_serde_roundtrip() {
        let a = EvoFabricAnchorCommitted::new([7u8; 32], 500, 10);
        let json = serde_json::to_string(&a).unwrap();
        let restored: EvoFabricAnchorCommitted = serde_json::from_str(&json).unwrap();
        assert_eq!(a.block_number, restored.block_number);
    }
}
