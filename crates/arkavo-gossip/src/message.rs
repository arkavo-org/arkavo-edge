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
    /// Announce availability of a context manifest (RLM mode)
    ContextManifestAnnounce(ContextManifestAnnouncement),
    /// Request specific chunks from a context manifest
    ContextChunkRequest(ContextChunkRequest),
    /// Deliver requested chunks from a context manifest
    ContextChunkDelivery(ContextChunkDelivery),
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

/// Announcement of a context manifest for RLM mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManifestAnnouncement {
    /// Unique manifest identifier
    pub manifest_id: String,
    /// Agent ID of the manifest holder
    pub holder: String,
    /// Total number of chunks in the manifest
    pub chunk_count: usize,
    /// Total token count across all chunks
    pub total_tokens: usize,
    /// Brief summary of the context content
    pub summary: String,
    /// When the manifest was created
    pub timestamp: DateTime<Utc>,
    /// TTL in seconds (how long this manifest will be available)
    pub ttl_seconds: u64,
    /// Ed25519 signature over the announcement
    pub signature: Vec<u8>,
}

impl ContextManifestAnnouncement {
    /// Create a new unsigned manifest announcement
    #[must_use]
    pub fn new(
        manifest_id: String,
        holder: String,
        chunk_count: usize,
        total_tokens: usize,
        summary: String,
        ttl_seconds: u64,
    ) -> Self {
        Self {
            manifest_id,
            holder,
            chunk_count,
            total_tokens,
            summary,
            timestamp: Utc::now(),
            ttl_seconds,
            signature: Vec::new(),
        }
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.manifest_id.as_bytes());
        content.extend(self.holder.as_bytes());
        content.extend(self.chunk_count.to_le_bytes());
        content.extend(self.total_tokens.to_le_bytes());
        content.extend(self.summary.as_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content.extend(self.ttl_seconds.to_le_bytes());
        content
    }
}

/// Request for specific chunks from a context manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunkRequest {
    /// Manifest ID being requested from
    pub manifest_id: String,
    /// Agent ID of the requester
    pub requester: String,
    /// Chunk indices to retrieve
    pub indices: Vec<usize>,
    /// Optional: keyword search instead of specific indices
    pub keywords: Option<Vec<String>>,
}

impl ContextChunkRequest {
    /// Create a new chunk request by indices
    #[must_use]
    pub fn by_indices(manifest_id: String, requester: String, indices: Vec<usize>) -> Self {
        Self {
            manifest_id,
            requester,
            indices,
            keywords: None,
        }
    }

    /// Create a new chunk request by keyword search
    #[must_use]
    pub fn by_keywords(manifest_id: String, requester: String, keywords: Vec<String>) -> Self {
        Self {
            manifest_id,
            requester,
            indices: Vec::new(),
            keywords: Some(keywords),
        }
    }
}

/// Delivery of requested chunks from a context manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunkDelivery {
    /// Manifest ID the chunks belong to
    pub manifest_id: String,
    /// Agent ID of the manifest holder
    pub holder: String,
    /// Delivered chunks with their indices and content
    pub chunks: Vec<ContextChunk>,
}

/// A single chunk from a context manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunk {
    /// Chunk index within the manifest
    pub index: usize,
    /// Estimated token count for this chunk
    pub tokens: usize,
    /// Full chunk content
    pub content: String,
    /// Content hints/keywords for searching
    pub hints: Vec<String>,
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

    #[test]
    fn test_context_manifest_announcement() {
        let ann = ContextManifestAnnouncement::new(
            "manifest-123".to_string(),
            "agent-1".to_string(),
            10,
            5000,
            "Test context summary".to_string(),
            3600,
        );

        assert!(ann.signature.is_empty());
        assert!(!ann.content_to_sign().is_empty());
        assert_eq!(ann.chunk_count, 10);
        assert_eq!(ann.total_tokens, 5000);
    }

    #[test]
    fn test_context_chunk_request() {
        let req = ContextChunkRequest::by_indices(
            "manifest-123".to_string(),
            "agent-2".to_string(),
            vec![0, 2, 5],
        );
        assert_eq!(req.indices.len(), 3);
        assert!(req.keywords.is_none());

        let search = ContextChunkRequest::by_keywords(
            "manifest-123".to_string(),
            "agent-2".to_string(),
            vec!["auth".to_string(), "login".to_string()],
        );
        assert!(search.indices.is_empty());
        assert_eq!(search.keywords.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_context_manifest_gossip_serialization() {
        let ann = ContextManifestAnnouncement::new(
            "manifest-456".to_string(),
            "agent-1".to_string(),
            5,
            2500,
            "Code review context".to_string(),
            1800,
        );

        let msg = GossipMessage::ContextManifestAnnounce(ann);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("context_manifest_announce"));

        let restored: GossipMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            restored,
            GossipMessage::ContextManifestAnnounce(_)
        ));
    }
}
