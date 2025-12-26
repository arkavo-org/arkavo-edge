//! Gossip message types for cross-swarm learning
//!
//! These messages enable agents to share lessons and coordinate
//! learning across a swarm using the gossip protocol.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Announcement of a new lesson to the swarm
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonAnnouncement {
    /// Unique lesson identifier
    pub lesson_id: Uuid,
    /// SHA-256 hash of the lesson content
    pub lesson_hash: [u8; 32],
    /// Agent ID of the originator
    pub originator: String,
    /// Swarm this lesson belongs to
    pub swarm_id: String,
    /// Category the lesson applies to
    pub category: String,
    /// Confidence in the lesson (0.0-1.0)
    pub confidence: f64,
    /// When the lesson was created
    pub timestamp: DateTime<Utc>,
    /// Ed25519 signature over the announcement
    pub signature: Vec<u8>,
}

impl LessonAnnouncement {
    /// Create a new unsigned announcement
    #[must_use]
    pub fn new(
        lesson_id: Uuid,
        lesson_hash: [u8; 32],
        originator: String,
        swarm_id: String,
        category: String,
        confidence: f64,
    ) -> Self {
        Self {
            lesson_id,
            lesson_hash,
            originator,
            swarm_id,
            category,
            confidence,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.lesson_id.as_bytes());
        content.extend(&self.lesson_hash);
        content.extend(self.originator.as_bytes());
        content.extend(self.swarm_id.as_bytes());
        content.extend(self.category.as_bytes());
        content.extend(self.confidence.to_le_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

/// Vote on a lesson's validity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonVote {
    /// Lesson being voted on
    pub lesson_id: Uuid,
    /// Agent ID of the voter
    pub voter: String,
    /// Whether to approve the lesson
    pub approve: bool,
    /// Optional local evidence supporting the vote
    pub local_evidence: Option<LocalEvidence>,
    /// Ed25519 signature over the vote
    pub signature: Vec<u8>,
    /// When the vote was cast
    pub voted_at: DateTime<Utc>,
}

impl LessonVote {
    /// Create a new unsigned vote
    #[must_use]
    pub fn new(lesson_id: Uuid, voter: String, approve: bool) -> Self {
        Self {
            lesson_id,
            voter,
            approve,
            local_evidence: None,
            signature: Vec::new(),
            voted_at: Utc::now(),
        }
    }

    /// Add local evidence
    pub fn with_evidence(mut self, evidence: LocalEvidence) -> Self {
        self.local_evidence = Some(evidence);
        self
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.lesson_id.as_bytes());
        content.extend(self.voter.as_bytes());
        content.push(if self.approve { 1 } else { 0 });
        content.extend(self.voted_at.timestamp().to_le_bytes());
        content
    }
}

/// Evidence from local episodes supporting a vote
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalEvidence {
    /// Number of local episodes supporting this lesson
    pub supporting_episodes: u32,
    /// Number of local episodes conflicting with this lesson
    pub conflicting_episodes: u32,
    /// Local confidence based on episodes
    pub local_confidence: f64,
}

impl LocalEvidence {
    /// Create new evidence
    pub fn new(supporting: u32, conflicting: u32) -> Self {
        let total = supporting + conflicting;
        let confidence = if total == 0 {
            0.5
        } else {
            f64::from(supporting) / f64::from(total)
        };
        Self {
            supporting_episodes: supporting,
            conflicting_episodes: conflicting,
            local_confidence: confidence,
        }
    }
}

/// Request for a specific lesson
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonRequest {
    /// Lesson ID being requested
    pub lesson_id: Uuid,
    /// Agent ID of the requester
    pub requester: String,
}

impl LessonRequest {
    /// Create a new request
    #[must_use]
    pub fn new(lesson_id: Uuid, requester: String) -> Self {
        Self {
            lesson_id,
            requester,
        }
    }
}

/// Full lesson delivery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonDelivery {
    /// Lesson identifier
    pub lesson_id: Uuid,
    /// The lesson content (serialized Lesson)
    pub content: Vec<u8>,
    /// Hash of the content for verification
    pub content_hash: [u8; 32],
    /// Collected votes for the lesson
    pub votes: Vec<LessonVote>,
}

impl LessonDelivery {
    /// Create a new delivery
    pub fn new(lesson_id: Uuid, content: Vec<u8>, content_hash: [u8; 32]) -> Self {
        Self {
            lesson_id,
            content,
            content_hash,
            votes: Vec::new(),
        }
    }

    /// Add votes
    pub fn with_votes(mut self, votes: Vec<LessonVote>) -> Self {
        self.votes = votes;
        self
    }
}

/// Anti-entropy digest for lesson consistency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonDigest {
    /// Agent ID sending the digest
    pub sender: String,
    /// Known lessons with their statuses
    pub known_lessons: Vec<LessonDigestEntry>,
    /// Timestamp of the digest
    pub timestamp: DateTime<Utc>,
}

impl LessonDigest {
    /// Create a new digest
    pub fn new(sender: String) -> Self {
        Self {
            sender,
            known_lessons: Vec::new(),
            timestamp: Utc::now(),
        }
    }

    /// Add a lesson entry
    pub fn add_lesson(&mut self, entry: LessonDigestEntry) {
        self.known_lessons.push(entry);
    }
}

/// Entry in lesson anti-entropy digest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonDigestEntry {
    /// Lesson ID
    pub lesson_id: Uuid,
    /// Lesson hash
    pub lesson_hash: [u8; 32],
    /// Current status
    pub status: LessonStatus,
}

impl LessonDigestEntry {
    /// Create a new entry
    pub fn new(lesson_id: Uuid, lesson_hash: [u8; 32], status: LessonStatus) -> Self {
        Self {
            lesson_id,
            lesson_hash,
            status,
        }
    }
}

/// Status of a lesson in the gossip protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonStatus {
    /// Lesson announced, waiting for votes
    Pending,
    /// Lesson approved by quorum
    Approved,
    /// Lesson rejected by quorum
    Rejected,
    /// Lesson applied locally
    Applied,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lesson_announcement() {
        let ann = LessonAnnouncement::new(
            Uuid::new_v4(),
            [0u8; 32],
            "agent-1".to_string(),
            "swarm-1".to_string(),
            "navigation".to_string(),
            0.9,
        );

        assert!(ann.signature.is_empty());
        assert!(!ann.content_to_sign().is_empty());
        assert_eq!(ann.confidence, 0.9);
    }

    #[test]
    fn test_lesson_vote() {
        let vote = LessonVote::new(Uuid::new_v4(), "voter-1".to_string(), true);

        assert!(vote.approve);
        assert!(vote.signature.is_empty());
        assert!(!vote.content_to_sign().is_empty());
    }

    #[test]
    fn test_local_evidence() {
        let evidence = LocalEvidence::new(8, 2);

        assert_eq!(evidence.supporting_episodes, 8);
        assert_eq!(evidence.conflicting_episodes, 2);
        assert!((evidence.local_confidence - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_lesson_digest() {
        let mut digest = LessonDigest::new("agent-1".to_string());

        digest.add_lesson(LessonDigestEntry::new(
            Uuid::new_v4(),
            [0u8; 32],
            LessonStatus::Approved,
        ));

        assert_eq!(digest.known_lessons.len(), 1);
        assert_eq!(digest.known_lessons[0].status, LessonStatus::Approved);
    }
}
