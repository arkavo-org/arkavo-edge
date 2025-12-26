//! Gossip message types for patch propagation and learning

use crate::learning_message::{
    LessonAnnouncement, LessonDelivery, LessonDigest, LessonRequest, LessonVote as LearningVote,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Gossip message types for patch propagation and learning
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GossipMessage {
    /// Announce a new patch
    PatchAnnounce(PatchAnnouncement),
    /// Vote on a patch
    PatchVote(PatchVote),
    /// Request a specific patch
    PatchRequest(PatchRequest),
    /// Deliver the full patch content
    PatchDelivery(PatchDelivery),
    /// Anti-entropy digest for consistency
    AntiEntropy(AntiEntropyDigest),
    /// Announce a new lesson to the swarm
    LessonAnnounce(LessonAnnouncement),
    /// Vote on a lesson
    LessonVote(LearningVote),
    /// Request a specific lesson
    LessonRequest(LessonRequest),
    /// Deliver the full lesson content
    LessonDelivery(LessonDelivery),
    /// Anti-entropy digest for lessons
    LessonDigest(LessonDigest),
}

/// Announcement of a new patch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchAnnouncement {
    /// Unique patch identifier
    pub patch_id: Uuid,
    /// SHA-256 hash of the patch content
    pub patch_hash: [u8; 32],
    /// Agent ID of the originator
    pub originator: String,
    /// When the patch was created
    pub timestamp: DateTime<Utc>,
    /// Ed25519 signature over the announcement
    pub signature: Vec<u8>,
    /// Patches that must be applied before this one
    pub dependencies: Vec<Uuid>,
}

impl PatchAnnouncement {
    /// Create a new unsigned announcement
    #[must_use]
    pub fn new(
        patch_id: Uuid,
        patch_hash: [u8; 32],
        originator: String,
        dependencies: Vec<Uuid>,
    ) -> Self {
        Self {
            patch_id,
            patch_hash,
            originator,
            timestamp: Utc::now(),
            signature: Vec::new(),
            dependencies,
        }
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.patch_id.as_bytes());
        content.extend(&self.patch_hash);
        content.extend(self.originator.as_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        for dep in &self.dependencies {
            content.extend(dep.as_bytes());
        }
        content
    }
}

/// Vote on a patch acceptance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchVote {
    /// Patch being voted on
    pub patch_id: Uuid,
    /// Agent ID of the voter
    pub voter: String,
    /// Whether to approve the patch
    pub approve: bool,
    /// Ed25519 signature over the vote
    pub signature: Vec<u8>,
    /// When the vote was cast
    pub voted_at: DateTime<Utc>,
}

impl PatchVote {
    /// Create a new unsigned vote
    #[must_use]
    pub fn new(patch_id: Uuid, voter: String, approve: bool) -> Self {
        Self {
            patch_id,
            voter,
            approve,
            signature: Vec::new(),
            voted_at: Utc::now(),
        }
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.patch_id.as_bytes());
        content.extend(self.voter.as_bytes());
        content.push(if self.approve { 1 } else { 0 });
        content.extend(self.voted_at.timestamp().to_le_bytes());
        content
    }
}

/// Request for a specific patch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchRequest {
    /// Patch ID being requested
    pub patch_id: Uuid,
    /// Agent ID of the requester
    pub requester: String,
}

/// Full patch delivery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchDelivery {
    /// Patch identifier
    pub patch_id: Uuid,
    /// The patch content (serialized patchlet)
    pub content: Vec<u8>,
    /// Hash of the content for verification
    pub content_hash: [u8; 32],
    /// Collected votes for the patch
    pub votes: Vec<PatchVote>,
}

/// Anti-entropy digest for consistency checking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntiEntropyDigest {
    /// Agent ID sending the digest
    pub sender: String,
    /// Known patch IDs with their statuses
    pub known_patches: Vec<PatchDigestEntry>,
    /// Timestamp of the digest
    pub timestamp: DateTime<Utc>,
}

/// Entry in anti-entropy digest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchDigestEntry {
    /// Patch ID
    pub patch_id: Uuid,
    /// Patch hash
    pub patch_hash: [u8; 32],
    /// Current status
    pub status: PatchStatus,
}

/// Status of a patch in the gossip protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchStatus {
    /// Patch announced, waiting for votes
    Pending,
    /// Patch approved by quorum
    Approved,
    /// Patch rejected by quorum
    Rejected,
    /// Patch applied locally
    Applied,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patch_announcement() {
        let ann = PatchAnnouncement::new(Uuid::new_v4(), [0u8; 32], "agent-1".into(), vec![]);

        assert!(ann.signature.is_empty());
        assert!(!ann.content_to_sign().is_empty());
    }

    #[test]
    fn test_patch_vote() {
        let vote = PatchVote::new(Uuid::new_v4(), "voter-1".into(), true);

        assert!(vote.approve);
        assert!(vote.signature.is_empty());
        assert!(!vote.content_to_sign().is_empty());
    }

    #[test]
    fn test_gossip_message_serialization() {
        let ann = PatchAnnouncement::new(Uuid::new_v4(), [0u8; 32], "agent-1".into(), vec![]);

        let msg = GossipMessage::PatchAnnounce(ann);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("patch_announce"));

        let restored: GossipMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(restored, GossipMessage::PatchAnnounce(_)));
    }
}
