//! Lesson message handlers for the gossip protocol
//!
//! Handles lesson announcements, votes, requests, deliveries, and digests.

use chrono::Utc;
use uuid::Uuid;

use crate::error::{GossipError, GossipResult};
use crate::learning_message::{
    LessonAnnouncement, LessonDelivery, LessonDigest, LessonDigestEntry, LessonRequest,
    LessonStatus, LessonVote,
};
use crate::lesson_consensus::{LessonConsensusState, LessonConsensusStatus};
use crate::message::GossipMessage;
use crate::protocol::{GossipProtocol, LessonState};

impl GossipProtocol {
    /// Handle a lesson announcement
    pub(crate) async fn handle_lesson_announce(
        &self,
        announcement: LessonAnnouncement,
    ) -> GossipResult<Vec<GossipMessage>> {
        let lesson_id = announcement.lesson_id;
        let now = Utc::now();

        tracing::info!(
            lesson_id = %lesson_id,
            originator = %announcement.originator,
            category = %announcement.category,
            confidence = announcement.confidence,
            "Received lesson announcement"
        );

        // Check for duplicate
        {
            let seen = self.seen_lesson_ids.read().await;
            if seen.contains_key(&lesson_id) {
                tracing::debug!(lesson_id = %lesson_id, "Duplicate lesson announcement ignored");
                return Err(GossipError::Duplicate(lesson_id));
            }
        }

        // Verify signature
        {
            let verifier = self.verifier.read().await;
            verifier.verify_lesson_announcement(&announcement)?;
        }

        // Mark as seen with timestamp
        self.seen_lesson_ids.write().await.insert(lesson_id, now);

        // Store the lesson and auto-vote approve (signature-verified = trusted)
        let mut consensus = LessonConsensusState::new(lesson_id);
        let auto_vote = LessonVote::new(lesson_id, self.agent_id.clone(), true);
        consensus.add_vote(auto_vote.clone());

        // Check quorum immediately (originator may have also voted via delivery)
        let peer_count = self.peers.read().await.len();
        consensus.check_quorum(peer_count + 1, &self.config.quorum);

        let approved = consensus.status == LessonConsensusStatus::Approved;
        let status = if approved {
            LessonStatus::Approved
        } else {
            LessonStatus::Pending
        };

        let state = LessonState {
            announcement: announcement.clone(),
            status,
            consensus,
            content: None,
            created_at: now,
        };
        self.lessons.write().await.insert(lesson_id, state);

        // If already approved, emit notification
        if approved {
            tracing::info!(
                lesson_id = %lesson_id,
                "Lesson auto-approved on receipt (signature verified)"
            );
            if let Some(tx) = &self.lesson_approved_tx {
                let _ = tx.send(announcement.clone());
            }
        }

        // Generate messages to propagate
        let mut messages = vec![
            // Request the lesson content
            GossipMessage::LessonRequest(LessonRequest::new(lesson_id, self.agent_id.clone())),
            // Propagate announcement to peers (gossip)
            GossipMessage::LessonAnnounce(announcement),
            // Propagate our approve vote
            GossipMessage::LessonVote(auto_vote),
        ];

        // If not yet approved, peers still need votes to reach quorum
        if approved {
            messages.truncate(2); // No need to propagate vote if already approved
        }

        Ok(messages)
    }

    /// Handle a lesson vote
    pub(crate) async fn handle_lesson_vote(
        &self,
        vote: LessonVote,
    ) -> GossipResult<Vec<GossipMessage>> {
        tracing::info!(
            lesson_id = %vote.lesson_id,
            voter = %vote.voter,
            approve = vote.approve,
            "Received lesson vote"
        );

        // Verify signature
        {
            let verifier = self.verifier.read().await;
            verifier.verify_lesson_vote(&vote)?;
        }

        // Update consensus
        let mut lessons = self.lessons.write().await;
        if let Some(state) = lessons.get_mut(&vote.lesson_id) {
            state.consensus.add_vote(vote.clone());

            // Check if quorum reached
            let peer_count = self.peers.read().await.len();
            state
                .consensus
                .check_quorum(peer_count + 1, &self.config.quorum);

            // Update status based on consensus
            match state.consensus.status {
                LessonConsensusStatus::Approved => {
                    if state.status != LessonStatus::Approved {
                        state.status = LessonStatus::Approved;
                        tracing::info!(
                            lesson_id = %vote.lesson_id,
                            originator = %state.announcement.originator,
                            category = %state.announcement.category,
                            votes = state.consensus.votes.len(),
                            "Lesson approved by consensus"
                        );
                        // Emit lesson approval notification
                        if let Some(tx) = &self.lesson_approved_tx {
                            let _ = tx.send(state.announcement.clone());
                        }
                    }
                }
                LessonConsensusStatus::Rejected | LessonConsensusStatus::TimedOut => {
                    state.status = LessonStatus::Rejected;
                    tracing::info!(
                        lesson_id = %vote.lesson_id,
                        status = ?state.consensus.status,
                        "Lesson rejected by consensus"
                    );
                }
                LessonConsensusStatus::Pending => {}
            }
        }

        // Propagate vote
        Ok(vec![GossipMessage::LessonVote(vote)])
    }

    /// Handle a lesson request
    pub(crate) async fn handle_lesson_request(
        &self,
        request: LessonRequest,
    ) -> GossipResult<Vec<GossipMessage>> {
        let lessons = self.lessons.read().await;

        if let Some(state) = lessons.get(&request.lesson_id)
            && let Some(content) = &state.content
        {
            // We have the content, send it
            let delivery = LessonDelivery::new(
                request.lesson_id,
                content.clone(),
                state.announcement.lesson_hash,
            )
            .with_votes(state.consensus.votes.values().cloned().collect());
            return Ok(vec![GossipMessage::LessonDelivery(delivery)]);
        }

        // We don't have it, propagate the request
        Ok(vec![GossipMessage::LessonRequest(request)])
    }

    /// Handle a lesson delivery
    pub(crate) async fn handle_lesson_delivery(
        &self,
        delivery: LessonDelivery,
    ) -> GossipResult<Vec<GossipMessage>> {
        // Verify content hash
        {
            let verifier = self.verifier.read().await;
            verifier.verify_content_hash(&delivery.content, &delivery.content_hash)?;
        }

        // Store the content
        let mut lessons = self.lessons.write().await;
        if let Some(state) = lessons.get_mut(&delivery.lesson_id) {
            state.content = Some(delivery.content);

            // Add any votes we didn't have
            for vote in delivery.votes {
                if !state.consensus.votes.contains_key(&vote.voter) {
                    // Verify vote signature before adding
                    let verifier = self.verifier.read().await;
                    if verifier.verify_lesson_vote(&vote).is_ok() {
                        state.consensus.add_vote(vote);
                    }
                }
            }
        }

        Ok(vec![])
    }

    /// Handle lesson anti-entropy digest
    pub(crate) async fn handle_lesson_digest(
        &self,
        digest: LessonDigest,
    ) -> GossipResult<Vec<GossipMessage>> {
        let lessons = self.lessons.read().await;
        let mut messages = Vec::new();

        // Check for lessons we have that they don't
        for (lesson_id, state) in lessons.iter() {
            let they_have = digest
                .known_lessons
                .iter()
                .any(|e| e.lesson_id == *lesson_id);

            if !they_have {
                messages.push(GossipMessage::LessonAnnounce(state.announcement.clone()));
            }
        }

        // Request lessons they have that we don't
        for entry in &digest.known_lessons {
            if !lessons.contains_key(&entry.lesson_id) {
                messages.push(GossipMessage::LessonRequest(LessonRequest::new(
                    entry.lesson_id,
                    self.agent_id.clone(),
                )));
            }
        }

        Ok(messages)
    }

    /// Create a lesson anti-entropy digest
    pub async fn create_lesson_digest(&self) -> LessonDigest {
        let lessons = self.lessons.read().await;

        let mut digest = LessonDigest::new(self.agent_id.clone());
        for state in lessons.values() {
            digest.add_lesson(LessonDigestEntry::new(
                state.announcement.lesson_id,
                state.announcement.lesson_hash,
                state.status,
            ));
        }

        digest
    }

    /// Get the status of a lesson
    pub async fn get_lesson_status(&self, lesson_id: Uuid) -> Option<LessonStatus> {
        self.lessons.read().await.get(&lesson_id).map(|s| s.status)
    }

    /// Cast a vote on a lesson
    pub async fn vote_lesson(&self, lesson_id: Uuid, approve: bool) -> GossipResult<LessonVote> {
        let lessons = self.lessons.read().await;
        if !lessons.contains_key(&lesson_id) {
            return Err(GossipError::LessonNotFound(lesson_id));
        }

        Ok(LessonVote::new(lesson_id, self.agent_id.clone(), approve))
    }

    /// Get count of tracked lessons
    pub async fn lesson_count(&self) -> usize {
        self.lessons.read().await.len()
    }
}
