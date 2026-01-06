//! Gossip protocol implementation
//!
//! Implements epidemic-style gossip for patch and lesson propagation across agents.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

use crate::consensus::{ConsensusState, ConsensusStatus, QuorumConfig};
use crate::error::{GossipError, GossipResult};
use crate::learning_message::{LessonAnnouncement, LessonStatus};
use crate::lesson_consensus::LessonConsensusState;
use crate::message::{
    AntiEntropyDigest, ContextChunkDelivery, ContextChunkRequest, ContextManifestAnnouncement,
    GossipMessage, PatchAnnouncement, PatchDelivery, PatchDigestEntry, PatchRequest, PatchStatus,
    PatchVote,
};
use crate::verification::{KeyRegistry, PatchVerifier};

/// Default gossip fanout (number of peers to propagate to)
pub const DEFAULT_FANOUT: usize = 3;

/// Default anti-entropy interval
pub const DEFAULT_ANTI_ENTROPY_INTERVAL: Duration = Duration::from_secs(30);

/// Configuration for the gossip protocol
#[derive(Debug, Clone)]
pub struct GossipConfig {
    /// Number of peers to propagate each message to
    pub fanout: usize,
    /// Quorum configuration for consensus
    pub quorum: QuorumConfig,
    /// Interval for anti-entropy synchronization
    pub anti_entropy_interval: Duration,
    /// Maximum message age before dropping
    pub max_message_age: Duration,
}

impl Default for GossipConfig {
    fn default() -> Self {
        Self {
            fanout: DEFAULT_FANOUT,
            quorum: QuorumConfig::default(),
            anti_entropy_interval: DEFAULT_ANTI_ENTROPY_INTERVAL,
            max_message_age: Duration::from_secs(300),
        }
    }
}

/// State for a tracked patch
#[derive(Debug, Clone)]
struct PatchState {
    /// The announcement
    announcement: PatchAnnouncement,
    /// Current status
    status: PatchStatus,
    /// Consensus state for voting
    consensus: ConsensusState,
    /// Patch content if received
    content: Option<Vec<u8>>,
    /// When this patch was first seen
    created_at: DateTime<Utc>,
}

/// State for a tracked lesson
#[derive(Debug, Clone)]
pub(crate) struct LessonState {
    /// The announcement
    pub(crate) announcement: LessonAnnouncement,
    /// Current status
    pub(crate) status: LessonStatus,
    /// Consensus state for voting
    pub(crate) consensus: LessonConsensusState,
    /// Lesson content if received
    pub(crate) content: Option<Vec<u8>>,
    /// When this lesson was first seen
    pub(crate) created_at: DateTime<Utc>,
}

/// Maximum size for seen_* sets before triggering cleanup
const MAX_SEEN_SIZE: usize = 10000;

/// Default max messages per peer per window
const DEFAULT_MAX_MESSAGES_PER_PEER: usize = 100;

/// Default rate limit window
const DEFAULT_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

/// The gossip protocol handler
pub struct GossipProtocol {
    /// Our agent ID
    pub(crate) agent_id: String,
    /// Protocol configuration
    pub(crate) config: GossipConfig,
    /// Known peers (peer_id -> ())
    pub(crate) peers: Arc<RwLock<HashMap<String, ()>>>,
    /// Tracked patches
    patches: Arc<RwLock<HashMap<Uuid, PatchState>>>,
    /// Tracked lessons
    pub(crate) lessons: Arc<RwLock<HashMap<Uuid, LessonState>>>,
    /// Message verifier
    pub(crate) verifier: Arc<RwLock<PatchVerifier>>,
    /// Seen patch IDs with timestamps (for deduplication with TTL)
    seen_messages: Arc<RwLock<HashMap<Uuid, DateTime<Utc>>>>,
    /// Seen lesson IDs with timestamps (for deduplication with TTL)
    pub(crate) seen_lesson_ids: Arc<RwLock<HashMap<Uuid, DateTime<Utc>>>>,
    /// Broadcast channel for lesson approval notifications
    pub(crate) lesson_approved_tx: Option<broadcast::Sender<LessonAnnouncement>>,
    /// Per-peer message timestamps for rate limiting
    peer_message_times: Arc<RwLock<HashMap<String, Vec<DateTime<Utc>>>>>,
    /// Max messages per peer per window
    max_messages_per_peer: usize,
    /// Window duration for rate limiting
    rate_limit_window: Duration,
}

impl GossipProtocol {
    /// Create a new gossip protocol handler
    pub fn new(agent_id: String, config: GossipConfig, key_registry: KeyRegistry) -> Self {
        Self {
            agent_id,
            config,
            peers: Arc::new(RwLock::new(HashMap::new())),
            patches: Arc::new(RwLock::new(HashMap::new())),
            lessons: Arc::new(RwLock::new(HashMap::new())),
            verifier: Arc::new(RwLock::new(PatchVerifier::new(key_registry))),
            seen_messages: Arc::new(RwLock::new(HashMap::new())),
            seen_lesson_ids: Arc::new(RwLock::new(HashMap::new())),
            lesson_approved_tx: None,
            peer_message_times: Arc::new(RwLock::new(HashMap::new())),
            max_messages_per_peer: DEFAULT_MAX_MESSAGES_PER_PEER,
            rate_limit_window: DEFAULT_RATE_LIMIT_WINDOW,
        }
    }

    /// Set the broadcast channel for lesson approval notifications
    pub fn set_lesson_approved_callback(&mut self, tx: broadcast::Sender<LessonAnnouncement>) {
        self.lesson_approved_tx = Some(tx);
    }

    /// Subscribe to lesson approval notifications
    pub fn subscribe_lesson_approvals(&self) -> Option<broadcast::Receiver<LessonAnnouncement>> {
        self.lesson_approved_tx.as_ref().map(|tx| tx.subscribe())
    }

    /// Add a peer to the known peers list
    pub async fn add_peer(&self, peer_id: String) {
        self.peers.write().await.insert(peer_id, ());
    }

    /// Remove a peer from the known peers list
    pub async fn remove_peer(&self, peer_id: &str) {
        self.peers.write().await.remove(peer_id);
    }

    /// Get number of known peers
    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    /// Check if a peer is rate limited
    ///
    /// Returns true if the peer is allowed to send, false if rate limited.
    pub async fn check_peer_rate_limit(&self, peer_id: &str) -> bool {
        let now = Utc::now();
        let window = chrono::Duration::from_std(self.rate_limit_window)
            .unwrap_or(chrono::Duration::seconds(60));

        let mut times = self.peer_message_times.write().await;
        let peer_times = times.entry(peer_id.to_string()).or_default();

        // Remove expired entries
        peer_times.retain(|t| now.signed_duration_since(*t) < window);

        // Check if under limit
        if peer_times.len() >= self.max_messages_per_peer {
            tracing::debug!(
                "Peer {} rate limited: {} messages in window",
                peer_id,
                peer_times.len()
            );
            false
        } else {
            peer_times.push(now);
            true
        }
    }

    /// Handle an incoming gossip message with rate limiting
    ///
    /// Returns Err(GossipError::RateLimited) if the peer has exceeded the rate limit.
    pub async fn handle_message_from_peer(
        &self,
        from_peer: &str,
        message: GossipMessage,
    ) -> GossipResult<Vec<GossipMessage>> {
        if !self.check_peer_rate_limit(from_peer).await {
            return Err(GossipError::RateLimited(from_peer.to_string()));
        }
        self.handle_message(message).await
    }

    /// Handle an incoming gossip message
    pub async fn handle_message(&self, message: GossipMessage) -> GossipResult<Vec<GossipMessage>> {
        match message {
            GossipMessage::PatchAnnounce(announcement) => {
                self.handle_announcement(announcement).await
            }
            GossipMessage::PatchVote(vote) => self.handle_vote(vote).await,
            GossipMessage::PatchRequest(request) => self.handle_request(request).await,
            GossipMessage::PatchDelivery(delivery) => self.handle_delivery(delivery).await,
            GossipMessage::AntiEntropy(digest) => self.handle_anti_entropy(digest).await,
            GossipMessage::LessonAnnounce(ann) => self.handle_lesson_announce(ann).await,
            GossipMessage::LessonVote(vote) => self.handle_lesson_vote(vote).await,
            GossipMessage::LessonRequest(req) => self.handle_lesson_request(req).await,
            GossipMessage::LessonDelivery(delivery) => self.handle_lesson_delivery(delivery).await,
            GossipMessage::LessonDigest(digest) => self.handle_lesson_digest(digest).await,
            // RLM context manifest messages
            GossipMessage::ContextManifestAnnounce(ann) => {
                self.handle_context_manifest_announce(ann).await
            }
            GossipMessage::ContextChunkRequest(req) => self.handle_context_chunk_request(req).await,
            GossipMessage::ContextChunkDelivery(delivery) => {
                self.handle_context_chunk_delivery(delivery).await
            }
        }
    }

    /// Handle a patch announcement
    async fn handle_announcement(
        &self,
        announcement: PatchAnnouncement,
    ) -> GossipResult<Vec<GossipMessage>> {
        let patch_id = announcement.patch_id;
        let now = Utc::now();

        // Check for duplicate
        {
            let seen = self.seen_messages.read().await;
            if seen.contains_key(&patch_id) {
                return Err(GossipError::Duplicate(patch_id));
            }
        }

        // Verify signature
        {
            let verifier = self.verifier.read().await;
            verifier.verify_announcement(&announcement)?;
        }

        // Mark as seen with timestamp
        self.seen_messages.write().await.insert(patch_id, now);

        // Store the patch
        let state = PatchState {
            announcement: announcement.clone(),
            status: PatchStatus::Pending,
            consensus: ConsensusState::new(patch_id),
            content: None,
            created_at: now,
        };
        self.patches.write().await.insert(patch_id, state);

        // Generate messages to propagate
        let messages = vec![
            // Request the patch content
            GossipMessage::PatchRequest(PatchRequest {
                patch_id,
                requester: self.agent_id.clone(),
            }),
            // Propagate announcement to peers (gossip)
            GossipMessage::PatchAnnounce(announcement),
        ];

        Ok(messages)
    }

    /// Handle a patch vote
    async fn handle_vote(&self, vote: PatchVote) -> GossipResult<Vec<GossipMessage>> {
        // Verify signature
        {
            let verifier = self.verifier.read().await;
            verifier.verify_vote(&vote)?;
        }

        // Update consensus
        let mut patches = self.patches.write().await;
        if let Some(state) = patches.get_mut(&vote.patch_id) {
            state.consensus.add_vote(vote.clone());

            // Check if quorum reached
            let peer_count = self.peers.read().await.len();
            state
                .consensus
                .check_quorum(peer_count + 1, &self.config.quorum);

            // Update status based on consensus
            match state.consensus.status {
                ConsensusStatus::Approved => {
                    state.status = PatchStatus::Approved;
                }
                ConsensusStatus::Rejected => {
                    state.status = PatchStatus::Rejected;
                }
                ConsensusStatus::TimedOut => {
                    state.status = PatchStatus::Rejected;
                }
                ConsensusStatus::Pending => {}
            }
        }

        // Propagate vote
        Ok(vec![GossipMessage::PatchVote(vote)])
    }

    /// Handle a patch request
    async fn handle_request(&self, request: PatchRequest) -> GossipResult<Vec<GossipMessage>> {
        let patches = self.patches.read().await;

        if let Some(state) = patches.get(&request.patch_id)
            && let Some(content) = &state.content
        {
            // We have the content, send it
            let delivery = PatchDelivery {
                patch_id: request.patch_id,
                content: content.clone(),
                content_hash: state.announcement.patch_hash,
                votes: state.consensus.votes.values().cloned().collect(),
            };
            return Ok(vec![GossipMessage::PatchDelivery(delivery)]);
        }

        // We don't have it, propagate the request
        Ok(vec![GossipMessage::PatchRequest(request)])
    }

    /// Handle a patch delivery
    async fn handle_delivery(&self, delivery: PatchDelivery) -> GossipResult<Vec<GossipMessage>> {
        // Verify content hash
        {
            let verifier = self.verifier.read().await;
            verifier.verify_content_hash(&delivery.content, &delivery.content_hash)?;
        }

        // Store the content
        let mut patches = self.patches.write().await;
        if let Some(state) = patches.get_mut(&delivery.patch_id) {
            state.content = Some(delivery.content);

            // Add any votes we didn't have
            for vote in delivery.votes {
                if !state.consensus.votes.contains_key(&vote.voter) {
                    // Verify vote signature before adding
                    let verifier = self.verifier.read().await;
                    if verifier.verify_vote(&vote).is_ok() {
                        state.consensus.add_vote(vote);
                    }
                }
            }
        }

        Ok(vec![])
    }

    /// Handle anti-entropy digest
    async fn handle_anti_entropy(
        &self,
        digest: AntiEntropyDigest,
    ) -> GossipResult<Vec<GossipMessage>> {
        let patches = self.patches.read().await;
        let mut messages = Vec::new();

        // Check for patches we have that they don't
        for (patch_id, state) in patches.iter() {
            let they_have = digest.known_patches.iter().any(|e| e.patch_id == *patch_id);

            if !they_have {
                // Send announcement
                messages.push(GossipMessage::PatchAnnounce(state.announcement.clone()));
            }
        }

        // Request patches they have that we don't
        for entry in &digest.known_patches {
            if !patches.contains_key(&entry.patch_id) {
                messages.push(GossipMessage::PatchRequest(PatchRequest {
                    patch_id: entry.patch_id,
                    requester: self.agent_id.clone(),
                }));
            }
        }

        Ok(messages)
    }

    /// Create an anti-entropy digest of our current state
    pub async fn create_digest(&self) -> AntiEntropyDigest {
        let patches = self.patches.read().await;

        let known_patches = patches
            .values()
            .map(|state| PatchDigestEntry {
                patch_id: state.announcement.patch_id,
                patch_hash: state.announcement.patch_hash,
                status: state.status,
            })
            .collect();

        AntiEntropyDigest {
            sender: self.agent_id.clone(),
            known_patches,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Get the status of a patch
    pub async fn get_patch_status(&self, patch_id: Uuid) -> Option<PatchStatus> {
        self.patches.read().await.get(&patch_id).map(|s| s.status)
    }

    /// Get peers to propagate a message to
    pub async fn select_propagation_peers(&self, exclude: Option<&str>) -> Vec<String> {
        let peers = self.peers.read().await;
        let mut selected: Vec<String> = peers
            .keys()
            .filter(|p| exclude.is_none_or(|e| p.as_str() != e))
            .cloned()
            .collect();

        // Shuffle and take up to fanout peers
        // Simple deterministic shuffle for testing
        selected.sort();
        selected.truncate(self.config.fanout);
        selected
    }

    /// Cast a vote on a patch
    pub async fn vote(&self, patch_id: Uuid, approve: bool) -> GossipResult<PatchVote> {
        let patches = self.patches.read().await;
        if !patches.contains_key(&patch_id) {
            return Err(GossipError::PatchNotFound(patch_id));
        }

        Ok(PatchVote::new(patch_id, self.agent_id.clone(), approve))
    }

    /// Get count of tracked patches
    pub async fn patch_count(&self) -> usize {
        self.patches.read().await.len()
    }

    /// Register a key for an agent
    pub async fn register_key(&self, agent_id: String, pubkey: arkavo_crypto::AgentPublicKey) {
        self.verifier
            .write()
            .await
            .registry_mut()
            .register(agent_id, pubkey);
    }

    /// Clean up expired entries based on max_message_age
    ///
    /// Removes patches, lessons, and seen entries older than the configured TTL.
    pub async fn cleanup_expired(&self) {
        let now = Utc::now();
        let max_age = chrono::Duration::from_std(self.config.max_message_age)
            .unwrap_or(chrono::Duration::seconds(300));

        // Clean expired patches
        {
            let mut patches = self.patches.write().await;
            let before = patches.len();
            patches.retain(|_, state| {
                let age = now.signed_duration_since(state.created_at);
                age < max_age
            });
            let removed = before - patches.len();
            if removed > 0 {
                tracing::debug!("Cleaned up {} expired patches", removed);
            }
        }

        // Clean expired lessons
        {
            let mut lessons = self.lessons.write().await;
            let before = lessons.len();
            lessons.retain(|_, state| {
                let age = now.signed_duration_since(state.created_at);
                age < max_age
            });
            let removed = before - lessons.len();
            if removed > 0 {
                tracing::debug!("Cleaned up {} expired lessons", removed);
            }
        }

        // Clean expired seen_messages
        {
            let mut seen = self.seen_messages.write().await;
            let before = seen.len();
            seen.retain(|_, timestamp| {
                let age = now.signed_duration_since(*timestamp);
                age < max_age
            });
            let removed = before - seen.len();
            if removed > 0 {
                tracing::debug!("Cleaned up {} expired seen_messages", removed);
            }

            // Also clear if exceeds max size
            if seen.len() > MAX_SEEN_SIZE {
                tracing::warn!(
                    "seen_messages exceeds max size ({}), clearing",
                    MAX_SEEN_SIZE
                );
                seen.clear();
            }
        }

        // Clean expired seen_lesson_ids
        {
            let mut seen = self.seen_lesson_ids.write().await;
            let before = seen.len();
            seen.retain(|_, timestamp| {
                let age = now.signed_duration_since(*timestamp);
                age < max_age
            });
            let removed = before - seen.len();
            if removed > 0 {
                tracing::debug!("Cleaned up {} expired seen_lesson_ids", removed);
            }

            // Also clear if exceeds max size
            if seen.len() > MAX_SEEN_SIZE {
                tracing::warn!(
                    "seen_lesson_ids exceeds max size ({}), clearing",
                    MAX_SEEN_SIZE
                );
                seen.clear();
            }
        }
    }

    /// Get cleanup statistics
    pub async fn stats(&self) -> GossipStats {
        GossipStats {
            peer_count: self.peers.read().await.len(),
            patch_count: self.patches.read().await.len(),
            lesson_count: self.lessons.read().await.len(),
            seen_messages_count: self.seen_messages.read().await.len(),
            seen_lesson_ids_count: self.seen_lesson_ids.read().await.len(),
        }
    }

    // ========== RLM Context Manifest Handlers ==========

    /// Handle a context manifest announcement
    ///
    /// When an agent announces a context manifest, other agents can later request chunks.
    async fn handle_context_manifest_announce(
        &self,
        announcement: ContextManifestAnnouncement,
    ) -> GossipResult<Vec<GossipMessage>> {
        tracing::info!(
            "Received context manifest announcement: {} ({} chunks, {} tokens) from {}",
            announcement.manifest_id,
            announcement.chunk_count,
            announcement.total_tokens,
            announcement.holder
        );

        // Propagate to peers (basic gossip)
        let mut messages = Vec::new();
        let peers = self
            .select_propagation_peers(Some(&announcement.holder))
            .await;

        if !peers.is_empty() {
            messages.push(GossipMessage::ContextManifestAnnounce(announcement));
        }

        Ok(messages)
    }

    /// Handle a context chunk request
    ///
    /// If we hold the manifest, return the requested chunks.
    async fn handle_context_chunk_request(
        &self,
        request: ContextChunkRequest,
    ) -> GossipResult<Vec<GossipMessage>> {
        tracing::debug!(
            "Received context chunk request: manifest={} from {} (indices={:?}, keywords={:?})",
            request.manifest_id,
            request.requester,
            request.indices,
            request.keywords
        );

        // NOTE: Actual chunk retrieval would integrate with RlmContextManager here.
        // For now, we log and return empty - the A2A RPC layer handles direct requests.
        Ok(Vec::new())
    }

    /// Handle a context chunk delivery
    ///
    /// Store the received chunks for local use.
    async fn handle_context_chunk_delivery(
        &self,
        delivery: ContextChunkDelivery,
    ) -> GossipResult<Vec<GossipMessage>> {
        tracing::debug!(
            "Received {} chunks for manifest {} from {}",
            delivery.chunks.len(),
            delivery.manifest_id,
            delivery.holder
        );

        // NOTE: Chunk storage would integrate with RlmContextManager here.
        // For now, we just acknowledge receipt.
        Ok(Vec::new())
    }
}

/// Statistics for the gossip protocol
#[derive(Debug, Clone)]
pub struct GossipStats {
    pub peer_count: usize,
    pub patch_count: usize,
    pub lesson_count: usize,
    pub seen_messages_count: usize,
    pub seen_lesson_ids_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::sign_announcement;
    use arkavo_crypto::AgentKeypair;

    fn create_test_protocol(agent_id: &str) -> GossipProtocol {
        let config = GossipConfig::default();
        let registry = KeyRegistry::new();
        GossipProtocol::new(agent_id.to_string(), config, registry)
    }

    #[tokio::test]
    async fn test_add_remove_peers() {
        let protocol = create_test_protocol("agent-1");

        assert_eq!(protocol.peer_count().await, 0);

        protocol.add_peer("peer-1".to_string()).await;
        protocol.add_peer("peer-2".to_string()).await;
        assert_eq!(protocol.peer_count().await, 2);

        protocol.remove_peer("peer-1").await;
        assert_eq!(protocol.peer_count().await, 1);
    }

    #[tokio::test]
    async fn test_handle_signed_announcement() {
        let protocol = create_test_protocol("agent-1");

        // Create and sign announcement
        let keypair = AgentKeypair::generate();
        protocol
            .register_key("originator".to_string(), keypair.public_key().clone())
            .await;

        let mut announcement =
            PatchAnnouncement::new(Uuid::new_v4(), [0u8; 32], "originator".to_string(), vec![]);
        sign_announcement(&mut announcement, &keypair).unwrap();

        let messages = protocol
            .handle_message(GossipMessage::PatchAnnounce(announcement.clone()))
            .await
            .unwrap();

        // Should generate request and propagation messages
        assert!(!messages.is_empty());

        // Patch should be tracked
        assert_eq!(protocol.patch_count().await, 1);
    }

    #[tokio::test]
    async fn test_duplicate_announcement() {
        let protocol = create_test_protocol("agent-1");

        let keypair = AgentKeypair::generate();
        protocol
            .register_key("originator".to_string(), keypair.public_key().clone())
            .await;

        let mut announcement =
            PatchAnnouncement::new(Uuid::new_v4(), [0u8; 32], "originator".to_string(), vec![]);
        sign_announcement(&mut announcement, &keypair).unwrap();

        // First should succeed
        protocol
            .handle_message(GossipMessage::PatchAnnounce(announcement.clone()))
            .await
            .unwrap();

        // Second should be duplicate
        let result = protocol
            .handle_message(GossipMessage::PatchAnnounce(announcement))
            .await;
        assert!(matches!(result, Err(GossipError::Duplicate(_))));
    }

    #[tokio::test]
    async fn test_create_digest() {
        let protocol = create_test_protocol("agent-1");

        let digest = protocol.create_digest().await;
        assert_eq!(digest.sender, "agent-1");
        assert!(digest.known_patches.is_empty());
    }

    #[tokio::test]
    async fn test_select_propagation_peers() {
        let protocol = create_test_protocol("agent-1");

        for i in 0..10 {
            protocol.add_peer(format!("peer-{}", i)).await;
        }

        let peers = protocol.select_propagation_peers(None).await;
        assert_eq!(peers.len(), DEFAULT_FANOUT);

        let peers = protocol.select_propagation_peers(Some("peer-0")).await;
        assert!(!peers.contains(&"peer-0".to_string()));
    }

    #[tokio::test]
    async fn test_vote_on_unknown_patch() {
        let protocol = create_test_protocol("agent-1");

        let result = protocol.vote(Uuid::new_v4(), true).await;
        assert!(matches!(result, Err(GossipError::PatchNotFound(_))));
    }
}
