//! Quorum consensus for lesson approval
//!
//! Lesson-specific consensus using LessonVote instead of PatchVote.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::consensus::QuorumConfig;
use crate::learning_message::LessonVote;

/// Current state of consensus for a lesson
#[derive(Debug, Clone)]
pub struct LessonConsensusState {
    /// Lesson being voted on
    pub lesson_id: Uuid,
    /// Collected votes by voter ID
    pub votes: HashMap<String, LessonVote>,
    /// When consensus started
    pub started_at: DateTime<Utc>,
    /// Current consensus status
    pub status: LessonConsensusStatus,
}

impl LessonConsensusState {
    /// Create a new consensus state for a lesson
    #[must_use]
    pub fn new(lesson_id: Uuid) -> Self {
        Self {
            lesson_id,
            votes: HashMap::new(),
            started_at: Utc::now(),
            status: LessonConsensusStatus::Pending,
        }
    }

    /// Add a vote to the consensus
    pub fn add_vote(&mut self, vote: LessonVote) {
        if self.status == LessonConsensusStatus::Pending {
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
        if self.status != LessonConsensusStatus::Pending {
            return;
        }

        let required = config.required_votes(total_voters);

        if self.approve_count() >= required {
            self.status = LessonConsensusStatus::Approved;
        } else if self.reject_count() > total_voters - required {
            self.status = LessonConsensusStatus::Rejected;
        }
    }

    /// Check if the consensus has timed out
    pub fn check_timeout(&mut self, config: &QuorumConfig) {
        if self.status != LessonConsensusStatus::Pending {
            return;
        }

        let elapsed = Utc::now()
            .signed_duration_since(self.started_at)
            .to_std()
            .unwrap_or(Duration::ZERO);

        if elapsed >= config.vote_timeout {
            self.status = LessonConsensusStatus::TimedOut;
        }
    }
}

/// Status of lesson consensus voting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonConsensusStatus {
    /// Voting in progress
    Pending,
    /// Quorum reached, lesson approved
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
    fn test_lesson_consensus_new() {
        let lesson_id = Uuid::new_v4();
        let state = LessonConsensusState::new(lesson_id);

        assert_eq!(state.lesson_id, lesson_id);
        assert!(state.votes.is_empty());
        assert_eq!(state.status, LessonConsensusStatus::Pending);
    }

    #[test]
    fn test_lesson_consensus_voting() {
        let lesson_id = Uuid::new_v4();
        let mut state = LessonConsensusState::new(lesson_id);
        let config = QuorumConfig::with_threshold(0.5);

        let vote = LessonVote::new(lesson_id, "voter-1".into(), true);
        state.add_vote(vote);
        assert_eq!(state.approve_count(), 1);
        assert_eq!(state.reject_count(), 0);

        state.check_quorum(2, &config);
        assert_eq!(state.status, LessonConsensusStatus::Approved);
    }

    #[test]
    fn test_lesson_consensus_rejection() {
        let lesson_id = Uuid::new_v4();
        let mut state = LessonConsensusState::new(lesson_id);
        let config = QuorumConfig::with_threshold(0.67);

        state.add_vote(LessonVote::new(lesson_id, "voter-1".into(), false));
        state.add_vote(LessonVote::new(lesson_id, "voter-2".into(), false));

        state.check_quorum(3, &config);
        assert_eq!(state.status, LessonConsensusStatus::Rejected);
    }

    #[test]
    fn test_lesson_consensus_no_duplicate_votes() {
        let lesson_id = Uuid::new_v4();
        let mut state = LessonConsensusState::new(lesson_id);

        state.add_vote(LessonVote::new(lesson_id, "voter-1".into(), true));
        state.add_vote(LessonVote::new(lesson_id, "voter-1".into(), false));

        assert_eq!(state.votes.len(), 1);
        assert_eq!(state.reject_count(), 1);
    }

    /// GOSSIP-006: With 3 voters and 0.67 threshold, exactly 3 approvals are
    /// needed. Two approvals stay Pending; the third tips to Approved.
    #[test]
    fn test_lesson_quorum_boundary() {
        let lesson_id = Uuid::new_v4();
        let config = QuorumConfig::default();

        assert_eq!(config.required_votes(3), 3);

        let mut state = LessonConsensusState::new(lesson_id);

        state.add_vote(LessonVote::new(lesson_id, "voter-1".into(), true));
        state.add_vote(LessonVote::new(lesson_id, "voter-2".into(), true));
        state.check_quorum(3, &config);
        assert_eq!(state.status, LessonConsensusStatus::Pending);

        state.add_vote(LessonVote::new(lesson_id, "voter-3".into(), true));
        state.check_quorum(3, &config);
        assert_eq!(state.status, LessonConsensusStatus::Approved);
    }

    /// GOSSIP-008: A pending lesson consensus that exceeds the vote timeout
    /// transitions to TimedOut.
    #[test]
    fn test_lesson_timeout() {
        let lesson_id = Uuid::new_v4();
        let config = QuorumConfig::default();
        let mut state = LessonConsensusState::new(lesson_id);

        state.started_at = Utc::now() - chrono::Duration::seconds(61);

        state.check_timeout(&config);
        assert_eq!(state.status, LessonConsensusStatus::TimedOut);
    }

    /// GOSSIP-006: A split vote (1 approve, 2 reject) with 3 voters should
    /// resolve to Rejected because approval is no longer reachable.
    #[test]
    fn test_lesson_split_vote() {
        let lesson_id = Uuid::new_v4();
        let config = QuorumConfig::default();
        let mut state = LessonConsensusState::new(lesson_id);

        state.add_vote(LessonVote::new(lesson_id, "voter-1".into(), true));
        state.add_vote(LessonVote::new(lesson_id, "voter-2".into(), false));
        state.add_vote(LessonVote::new(lesson_id, "voter-3".into(), false));

        state.check_quorum(3, &config);
        assert_eq!(state.status, LessonConsensusStatus::Rejected);
    }
}
