//! Gossip message types for distributed experiment coordination
//!
//! Enables nodes to share autoresearch experiment results and coordinate
//! parameter exploration across the swarm. Follows the same patterns as
//! LessonAnnouncement: signed announcements, quorum voting, anti-entropy.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Announcement of an experiment result to the swarm
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentAnnouncement {
    /// Unique experiment identifier
    pub experiment_id: Uuid,
    /// SHA-256 hash of the full result payload
    pub result_hash: [u8; 32],
    /// Agent/node ID of the originator
    pub originator: String,
    /// Hardware capability tier (for weighted aggregation)
    pub hardware_tier: HardwareTier,
    /// Configuration vector that was tested (serialized ExperimentConfig)
    pub config_json: String,
    /// Which model was tested (e.g. "qwen3.5-0.8b")
    #[serde(default)]
    pub model_name: String,
    /// Winning metric value (weighted_quality)
    pub weighted_quality: f64,
    /// Baseline tok/s on this hardware (for normalization)
    pub baseline_tok_per_sec: f64,
    /// Whether this config was kept (improvement) or reverted
    pub kept: bool,
    /// Thompson Sampling prior state for this arm (alpha, beta)
    pub prior_alpha: f64,
    /// Thompson Sampling prior state for this arm
    pub prior_beta: f64,
    /// When the experiment completed
    pub timestamp: DateTime<Utc>,
    /// Ed25519 signature over the announcement
    pub signature: Vec<u8>,
}

/// Metrics from an experiment run (groups related fields to keep arg count low)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentMetrics {
    /// Weighted quality metric value
    pub weighted_quality: f64,
    /// Baseline tok/s on the originating hardware
    pub baseline_tok_per_sec: f64,
    /// Whether this config was kept (improvement) or reverted
    pub kept: bool,
}

impl ExperimentAnnouncement {
    /// Create a new unsigned announcement
    #[must_use]
    pub fn new(
        experiment_id: Uuid,
        result_hash: [u8; 32],
        originator: String,
        hardware_tier: HardwareTier,
        config_json: String,
        model_name: String,
        metrics: ExperimentMetrics,
    ) -> Self {
        Self {
            experiment_id,
            result_hash,
            originator,
            hardware_tier,
            config_json,
            model_name,
            weighted_quality: metrics.weighted_quality,
            baseline_tok_per_sec: metrics.baseline_tok_per_sec,
            kept: metrics.kept,
            prior_alpha: 1.0,
            prior_beta: 1.0,
            timestamp: Utc::now(),
            signature: Vec::new(),
        }
    }

    /// Attach Thompson Sampling prior state
    #[must_use]
    pub fn with_prior(mut self, alpha: f64, beta: f64) -> Self {
        self.prior_alpha = alpha;
        self.prior_beta = beta;
        self
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.experiment_id.as_bytes());
        content.extend(&self.result_hash);
        content.extend(self.originator.as_bytes());
        content.push(self.hardware_tier as u8);
        content.extend(self.weighted_quality.to_le_bytes());
        content.extend(self.baseline_tok_per_sec.to_le_bytes());
        content.push(u8::from(self.kept));
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content
    }
}

/// Vote on an experiment result's validity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentVote {
    /// Experiment being voted on
    pub experiment_id: Uuid,
    /// Agent ID of the voter
    pub voter: String,
    /// Whether to accept the result
    pub accept: bool,
    /// Voter's local measurement if they reproduced the experiment
    pub reproduced_quality: Option<f64>,
    /// Ed25519 signature over the vote
    pub signature: Vec<u8>,
    /// When the vote was cast
    pub voted_at: DateTime<Utc>,
}

impl ExperimentVote {
    /// Create a new unsigned vote
    #[must_use]
    pub fn new(experiment_id: Uuid, voter: String, accept: bool) -> Self {
        Self {
            experiment_id,
            voter,
            accept,
            reproduced_quality: None,
            signature: Vec::new(),
            voted_at: Utc::now(),
        }
    }

    /// Attach reproduced measurement
    #[must_use]
    pub fn with_reproduction(mut self, quality: f64) -> Self {
        self.reproduced_quality = Some(quality);
        self
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.experiment_id.as_bytes());
        content.extend(self.voter.as_bytes());
        content.push(u8::from(self.accept));
        content.extend(self.voted_at.timestamp().to_le_bytes());
        content
    }
}

/// Hardware capability tier for weighted metric normalization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum HardwareTier {
    /// Raspberry Pi / edge device (< 8GB RAM)
    Edge = 0,
    /// Laptop / desktop (8-16GB, no discrete GPU)
    Desktop = 1,
    /// Workstation with GPU (16GB+ VRAM)
    Workstation = 2,
    /// Server with multiple GPUs
    Server = 3,
}

impl HardwareTier {
    /// Weight for cross-hardware metric normalization
    /// Higher tier = results are less surprising (lower weight)
    pub fn normalization_weight(self) -> f64 {
        match self {
            Self::Edge => 2.0,
            Self::Desktop => 1.0,
            Self::Workstation => 0.7,
            Self::Server => 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(wq: f64, tok_per_sec: f64, kept: bool) -> ExperimentMetrics {
        ExperimentMetrics {
            weighted_quality: wq,
            baseline_tok_per_sec: tok_per_sec,
            kept,
        }
    }

    #[test]
    fn test_experiment_announcement_new() {
        let ann = ExperimentAnnouncement::new(
            Uuid::new_v4(),
            [0u8; 32],
            "node-1".into(),
            HardwareTier::Desktop,
            r#"{"enable_thinking":false}"#.into(),
            "qwen3.5-0.8b".into(),
            metrics(0.23, 175.0, true),
        );
        assert!(ann.signature.is_empty());
        assert!(ann.kept);
        assert!((ann.prior_alpha - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_announcement_with_prior() {
        let ann = ExperimentAnnouncement::new(
            Uuid::new_v4(),
            [0u8; 32],
            "n".into(),
            HardwareTier::Edge,
            "{}".into(),
            "test-model".into(),
            metrics(0.1, 50.0, false),
        )
        .with_prior(5.0, 3.0);
        assert!((ann.prior_alpha - 5.0).abs() < f64::EPSILON);
        assert!((ann.prior_beta - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_content_to_sign_deterministic() {
        let ann = ExperimentAnnouncement::new(
            Uuid::new_v4(),
            [1u8; 32],
            "n".into(),
            HardwareTier::Desktop,
            "{}".into(),
            "test-model".into(),
            metrics(0.5, 100.0, true),
        );
        assert_eq!(ann.content_to_sign(), ann.content_to_sign());
    }

    #[test]
    fn test_content_to_sign_changes_with_quality() {
        let id = Uuid::new_v4();
        let mut a = ExperimentAnnouncement::new(
            id,
            [0u8; 32],
            "n".into(),
            HardwareTier::Desktop,
            "{}".into(),
            "test-model".into(),
            metrics(0.5, 100.0, true),
        );
        let mut b = ExperimentAnnouncement::new(
            id,
            [0u8; 32],
            "n".into(),
            HardwareTier::Desktop,
            "{}".into(),
            "test-model".into(),
            metrics(0.9, 100.0, true),
        );
        b.timestamp = a.timestamp;
        a.timestamp = b.timestamp;
        assert_ne!(a.content_to_sign(), b.content_to_sign());
    }

    #[test]
    fn test_experiment_vote() {
        let vote = ExperimentVote::new(Uuid::new_v4(), "voter-1".into(), true);
        assert!(vote.accept);
        assert!(vote.reproduced_quality.is_none());
        assert!(!vote.content_to_sign().is_empty());
    }

    #[test]
    fn test_vote_with_reproduction() {
        let vote = ExperimentVote::new(Uuid::new_v4(), "v".into(), true).with_reproduction(0.22);
        assert!((vote.reproduced_quality.unwrap() - 0.22).abs() < f64::EPSILON);
    }

    #[test]
    fn test_vote_accept_vs_reject_differ() {
        let id = Uuid::new_v4();
        let mut accept = ExperimentVote::new(id, "v".into(), true);
        let mut reject = ExperimentVote::new(id, "v".into(), false);
        reject.voted_at = accept.voted_at;
        accept.voted_at = reject.voted_at;
        assert_ne!(accept.content_to_sign(), reject.content_to_sign());
    }

    #[test]
    fn test_hardware_tier_weights() {
        assert!(
            HardwareTier::Edge.normalization_weight() > HardwareTier::Server.normalization_weight()
        );
        assert!((HardwareTier::Desktop.normalization_weight() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hardware_tier_serde() {
        for tier in [
            HardwareTier::Edge,
            HardwareTier::Desktop,
            HardwareTier::Workstation,
            HardwareTier::Server,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let back: HardwareTier = serde_json::from_str(&json).unwrap();
            assert_eq!(tier, back);
        }
    }
}
