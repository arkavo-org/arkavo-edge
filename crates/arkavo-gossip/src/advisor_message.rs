//! Gossip message types for advisor adjustment propagation
//!
//! Enables proven prompt fixes (advisor adjustments) to flow across
//! mesh agents automatically via the gossip protocol.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Quality statistics for an advisor adjustment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjustmentStats {
    /// Success rate from originator's feedback (0.0-1.0)
    pub success_rate: f64,
    /// Number of feedback observations
    pub feedback_count: u32,
    /// Number of times the adjustment was applied
    pub applications: u32,
    /// When the adjustment was last updated
    pub updated_at: DateTime<Utc>,
}

/// Announcement of a proven advisor adjustment to the mesh
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorAdjustmentAnnouncement {
    /// Unique announcement identifier
    pub announcement_id: Uuid,
    /// Agent ID of the originator
    pub originator: String,
    /// Model family the adjustment targets (e.g. "glm", "qwen")
    pub model_family: String,
    /// Issue type (e.g. "OutputLoop", "Timeout")
    pub issue: String,
    /// Human-readable label for the adjustment
    pub label: String,
    /// The prompt injection text
    pub text: String,
    /// Quality statistics
    pub stats: AdjustmentStats,
    /// When this announcement was created
    pub timestamp: DateTime<Utc>,
    /// Ed25519 signature over the announcement
    pub signature: Vec<u8>,
}

impl AdvisorAdjustmentAnnouncement {
    /// Create a new unsigned announcement
    #[must_use]
    pub fn new(
        originator: String,
        model_family: String,
        issue: String,
        label: String,
        text: String,
        stats: AdjustmentStats,
    ) -> Self {
        Self {
            announcement_id: Uuid::new_v4(),
            originator,
            model_family,
            issue,
            label,
            text,
            stats,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.announcement_id.as_bytes());
        content.extend(self.originator.as_bytes());
        content.extend(self.model_family.as_bytes());
        content.extend(self.issue.as_bytes());
        content.extend(self.label.as_bytes());
        content.extend(self.text.as_bytes());
        content.extend(self.stats.success_rate.to_le_bytes());
        content.extend(self.stats.feedback_count.to_le_bytes());
        content.extend(self.stats.applications.to_le_bytes());
        content.extend(self.stats.updated_at.timestamp().to_le_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_announcement() -> AdvisorAdjustmentAnnouncement {
        AdvisorAdjustmentAnnouncement::new(
            "commander".into(),
            "glm".into(),
            "Timeout".into(),
            "glm-timeout-dynamic".into(),
            "Keep your response brief.".into(),
            AdjustmentStats {
                success_rate: 0.99,
                feedback_count: 290,
                applications: 150,
                updated_at: Utc::now(),
            },
        )
    }

    #[test]
    fn test_announcement_creation() {
        let ann = make_announcement();
        assert!(ann.signature.is_empty());
        assert_eq!(ann.originator, "commander");
        assert_eq!(ann.model_family, "glm");
        assert_eq!(ann.issue, "Timeout");
    }

    #[test]
    fn test_content_to_sign_deterministic() {
        let ann = make_announcement();
        assert_eq!(ann.content_to_sign(), ann.content_to_sign());
    }

    #[test]
    fn test_content_to_sign_differs_by_field() {
        let a = make_announcement();
        let mut b = a.clone();
        b.stats.success_rate = 0.5;
        assert_ne!(a.content_to_sign(), b.content_to_sign());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let ann = make_announcement();
        let json = serde_json::to_string(&ann).unwrap();
        let restored: AdvisorAdjustmentAnnouncement = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.originator, ann.originator);
        assert_eq!(restored.model_family, ann.model_family);
        assert_eq!(restored.issue, ann.issue);
        assert!((restored.stats.success_rate - ann.stats.success_rate).abs() < f64::EPSILON);
    }
}
