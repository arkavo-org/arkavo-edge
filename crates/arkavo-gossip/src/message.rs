//! Gossip message types for patch propagation and learning

pub use crate::advisor_message::AdvisorAdjustmentAnnouncement;
pub use crate::context_message::{
    ContextChunkDelivery, ContextChunkRequest, ContextManifestAnnouncement,
};
use crate::evofabric_message::{
    EvoFabricAnchorCommitted, EvoFabricConflictNotice, EvoFabricMergeDecision, EvoFabricProposal,
    EvoFabricVerification,
};
use crate::experiment_message::{ExperimentAnnouncement, ExperimentVote};
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
    /// Announce a proven advisor adjustment to the mesh
    AdvisorAdjustmentAnnounce(AdvisorAdjustmentAnnouncement),
    /// Announce an autoresearch experiment result
    ExperimentAnnounce(ExperimentAnnouncement),
    /// Vote on an experiment result's validity
    ExperimentVote(ExperimentVote),
    /// Propose an EvoFabric OpBundle for AST mutation
    EvoFabricPropose(EvoFabricProposal),
    /// Verification result for an EvoFabric OpBundle
    EvoFabricVerify(EvoFabricVerification),
    /// Merge decision for an EvoFabric OpBundle
    EvoFabricMerge(EvoFabricMergeDecision),
    /// Conflict notice between two EvoFabric OpBundles
    EvoFabricConflict(EvoFabricConflictNotice),
    /// Merkle root anchored on-chain for EvoFabric
    EvoFabricAnchor(EvoFabricAnchorCommitted),
    /// Task completion notification from specialist to commander
    TaskCompleted(TaskCompletionNotice),
    /// GPU inference state broadcast for distributed scheduling
    InferenceState(InferenceStateBroadcast),
    /// Data artifact available on the Iroh data plane
    DataAvailable(DataAvailableAnnounce),
}

/// Announcement that a data artifact is available for retrieval via Iroh.
///
/// Generic: any agent can announce any data blob. The content_type field
/// tells receivers how to interpret the data (e.g. "episode_summary",
/// "observation_batch", "checkpoint"). Domain-specific interpretation
/// happens at the agent level, not in core crates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataAvailableAnnounce {
    /// Agent that staged the data
    pub originator: String,
    /// Iroh ticket string for data retrieval
    pub iroh_ticket: String,
    /// Content type hint (e.g. "episode_summary", "observation_batch")
    pub content_type: String,
    /// Optional metadata (reward totals, step counts, etc.)
    pub metadata: Option<serde_json::Value>,
    /// When the data was staged
    pub timestamp: DateTime<Utc>,
}

/// GPU inference activity broadcast.
///
/// Agents emit this on state transitions (idle→running, running→idle) so
/// peers on the same device can coordinate GPU access without a central
/// scheduler. Stale entries (timestamp > 30s) are expired by receivers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceStateBroadcast {
    /// Agent emitting this state
    pub agent_id: String,
    /// Number of active GPU inferences (u32, not bool — captures concurrent calls)
    pub active_count: u32,
    /// Model currently loaded/running
    pub model_name: String,
    /// Remaining inference budget for this agent
    pub budget_remaining: u32,
    /// Device identifier for multi-device filtering
    pub device_id: String,
    /// Timestamp for stale entry expiry
    pub timestamp: DateTime<Utc>,
}

/// Push notification when a specialist finishes a delegated task.
/// Sent over gossip so the commander doesn't need to poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompletionNotice {
    /// The task_id that was delegated via send_task
    pub task_id: String,
    /// Agent ID of the specialist that completed the task
    pub specialist_id: String,
    /// Whether the task succeeded or failed
    pub succeeded: bool,
    /// Result text (on success) or error message (on failure)
    pub content: String,
    /// Budget snapshot from the specialist, bundled to avoid a separate round-trip
    pub budget_snapshot: Option<serde_json::Value>,
    /// Wall-clock time from task receipt to completion
    pub completion_ms: u64,
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
    fn test_patch_announcement_content_includes_deps() {
        let dep = Uuid::new_v4();
        let a1 = PatchAnnouncement::new(Uuid::new_v4(), [0u8; 32], "a".into(), vec![]);
        let mut a2 = a1.clone();
        a2.dependencies = vec![dep];
        assert_ne!(a1.content_to_sign(), a2.content_to_sign());
    }

    #[test]
    fn test_patch_vote() {
        let vote = PatchVote::new(Uuid::new_v4(), "voter-1".into(), true);
        assert!(vote.approve);
        assert!(vote.signature.is_empty());
        assert!(!vote.content_to_sign().is_empty());
    }

    #[test]
    fn test_patch_vote_approve_vs_reject() {
        let id = Uuid::new_v4();
        let approve = PatchVote::new(id, "v".into(), true);
        let mut reject = approve.clone();
        reject.approve = false;
        assert_ne!(approve.content_to_sign(), reject.content_to_sign());
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
    fn test_patch_status_serialization() {
        for status in [
            PatchStatus::Pending,
            PatchStatus::Approved,
            PatchStatus::Rejected,
            PatchStatus::Applied,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let restored: PatchStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, restored);
        }
    }

    #[test]
    fn test_context_manifest_gossip_serialization() {
        let ann = crate::context_message::ContextManifestAnnouncement::new(
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
