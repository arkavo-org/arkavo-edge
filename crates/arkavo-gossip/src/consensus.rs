//! Quorum consensus for patch approval

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::message::PatchVote;

/// Default quorum threshold (2/3)
pub const DEFAULT_QUORUM_THRESHOLD: f64 = 0.67;

/// Default vote timeout
pub const DEFAULT_VOTE_TIMEOUT: Duration = Duration::from_secs(60);

/// Configuration for quorum consensus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumConfig {
    /// Threshold for approval (0.0-1.0)
    pub threshold: f64,
    /// Minimum number of voters required
    pub min_voters: usize,
    /// Timeout for vote collection
    pub vote_timeout: Duration,
}

impl Default for QuorumConfig {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_QUORUM_THRESHOLD,
            min_voters: 1,
            vote_timeout: DEFAULT_VOTE_TIMEOUT,
        }
    }
}

impl QuorumConfig {
    /// Create a new quorum config with custom threshold
    #[must_use]
    pub fn with_threshold(threshold: f64) -> Self {
        Self {
            threshold,
            ..Default::default()
        }
    }

    /// Calculate required votes for the given total
    #[must_use]
    pub fn required_votes(&self, total_voters: usize) -> usize {
        let required = (total_voters as f64 * self.threshold).ceil() as usize;
        required.max(self.min_voters)
    }
}

/// Current state of consensus for a patch
#[derive(Debug, Clone)]
pub struct ConsensusState {
    /// Patch being voted on
    pub patch_id: Uuid,
    /// Collected votes by voter ID
    pub votes: HashMap<String, PatchVote>,
    /// When consensus started
    pub started_at: DateTime<Utc>,
    /// Current consensus status
    pub status: ConsensusStatus,
}

impl ConsensusState {
    /// Create a new consensus state for a patch
    #[must_use]
    pub fn new(patch_id: Uuid) -> Self {
        Self {
            patch_id,
            votes: HashMap::new(),
            started_at: Utc::now(),
            status: ConsensusStatus::Pending,
        }
    }

    /// Add a vote to the consensus
    pub fn add_vote(&mut self, vote: PatchVote) {
        if self.status == ConsensusStatus::Pending {
            self.votes.insert(vote.voter.clone(), vote);
        }
    }

    /// Count approval votes
    #[must_use]
    pub fn approve_count(&self) -> usize {
        self.votes.values().filter(|v| v.approve).count()
    }

    /// Count rejection votes
    #[must_use]
    pub fn reject_count(&self) -> usize {
        self.votes.values().filter(|v| !v.approve).count()
    }

    /// Check if quorum has been reached
    pub fn check_quorum(&mut self, total_voters: usize, config: &QuorumConfig) {
        if self.status != ConsensusStatus::Pending {
            return;
        }

        let required = config.required_votes(total_voters);

        if self.approve_count() >= required {
            self.status = ConsensusStatus::Approved;
        } else if self.reject_count() > total_voters - required {
            // Cannot possibly reach approval anymore
            self.status = ConsensusStatus::Rejected;
        }
    }

    /// Check if the consensus has timed out
    pub fn check_timeout(&mut self, config: &QuorumConfig) {
        if self.status != ConsensusStatus::Pending {
            return;
        }

        let elapsed = Utc::now()
            .signed_duration_since(self.started_at)
            .to_std()
            .unwrap_or(Duration::ZERO);

        if elapsed >= config.vote_timeout {
            self.status = ConsensusStatus::TimedOut;
        }
    }
}

/// Status of consensus voting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsensusStatus {
    /// Voting in progress
    Pending,
    /// Quorum reached, patch approved
    Approved,
    /// Quorum not reached or rejected
    Rejected,
    /// Voting period expired
    TimedOut,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quorum_config_default() {
        let config = QuorumConfig::default();
        assert!((config.threshold - DEFAULT_QUORUM_THRESHOLD).abs() < f64::EPSILON);
        assert_eq!(config.min_voters, 1);
    }

    #[test]
    fn test_required_votes() {
        let config = QuorumConfig::default(); // 0.67 threshold

        // 3 voters: ceil(3 * 0.67) = ceil(2.01) = 3
        assert_eq!(config.required_votes(3), 3);

        // 10 voters: ceil(10 * 0.67) = ceil(6.7) = 7
        assert_eq!(config.required_votes(10), 7);

        // 1 voter: ceil(0.67) = 1
        assert_eq!(config.required_votes(1), 1);
    }

    #[test]
    fn test_consensus_state_new() {
        let patch_id = Uuid::new_v4();
        let state = ConsensusState::new(patch_id);

        assert_eq!(state.patch_id, patch_id);
        assert!(state.votes.is_empty());
        assert_eq!(state.status, ConsensusStatus::Pending);
    }

    #[test]
    fn test_consensus_voting() {
        let patch_id = Uuid::new_v4();
        let mut state = ConsensusState::new(patch_id);
        let config = QuorumConfig::with_threshold(0.5);

        // Add approval vote
        let vote1 = PatchVote::new(patch_id, "voter-1".into(), true);
        state.add_vote(vote1);
        assert_eq!(state.approve_count(), 1);
        assert_eq!(state.reject_count(), 0);

        // Check quorum with 2 voters (need 1 vote with 0.5 threshold)
        state.check_quorum(2, &config);
        assert_eq!(state.status, ConsensusStatus::Approved);
    }

    #[test]
    fn test_consensus_rejection() {
        let patch_id = Uuid::new_v4();
        let mut state = ConsensusState::new(patch_id);
        let config = QuorumConfig::with_threshold(0.67);

        // Add 2 rejection votes out of 3 total
        state.add_vote(PatchVote::new(patch_id, "voter-1".into(), false));
        state.add_vote(PatchVote::new(patch_id, "voter-2".into(), false));

        // With 2 rejections, cannot possibly reach 3 approvals (needed for 0.67 of 3)
        state.check_quorum(3, &config);
        assert_eq!(state.status, ConsensusStatus::Rejected);
    }

    #[test]
    fn test_consensus_no_duplicate_votes() {
        let patch_id = Uuid::new_v4();
        let mut state = ConsensusState::new(patch_id);

        state.add_vote(PatchVote::new(patch_id, "voter-1".into(), true));
        state.add_vote(PatchVote::new(patch_id, "voter-1".into(), false));

        // Same voter should only count once (last vote wins)
        assert_eq!(state.votes.len(), 1);
        // The last vote was false
        assert_eq!(state.reject_count(), 1);
    }
}
