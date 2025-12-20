//! Gossip protocol implementation
//!
//! Implements epidemic-style gossip for patch propagation across agents.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::consensus::{ConsensusState, ConsensusStatus, QuorumConfig};
use crate::error::{GossipError, GossipResult};
use crate::message::{
    AntiEntropyDigest, GossipMessage, PatchAnnouncement, PatchDelivery, PatchDigestEntry,
    PatchRequest, PatchStatus, PatchVote,
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
}

/// The gossip protocol handler
pub struct GossipProtocol {
    /// Our agent ID
    agent_id: String,
    /// Protocol configuration
    config: GossipConfig,
    /// Known peers
    peers: Arc<RwLock<HashSet<String>>>,
    /// Tracked patches
    patches: Arc<RwLock<HashMap<Uuid, PatchState>>>,
    /// Message verifier
    verifier: Arc<RwLock<PatchVerifier>>,
    /// Seen message IDs (for deduplication)
    seen_messages: Arc<RwLock<HashSet<Uuid>>>,
}

impl GossipProtocol {
    /// Create a new gossip protocol handler
    pub fn new(agent_id: String, config: GossipConfig, key_registry: KeyRegistry) -> Self {
        Self {
            agent_id,
            config,
            peers: Arc::new(RwLock::new(HashSet::new())),
            patches: Arc::new(RwLock::new(HashMap::new())),
            verifier: Arc::new(RwLock::new(PatchVerifier::new(key_registry))),
            seen_messages: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Add a peer to the known peers list
    pub async fn add_peer(&self, peer_id: String) {
        self.peers.write().await.insert(peer_id);
    }

    /// Remove a peer from the known peers list
    pub async fn remove_peer(&self, peer_id: &str) {
        self.peers.write().await.remove(peer_id);
    }

    /// Get number of known peers
    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
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
        }
    }

    /// Handle a patch announcement
    async fn handle_announcement(
        &self,
        announcement: PatchAnnouncement,
    ) -> GossipResult<Vec<GossipMessage>> {
        let patch_id = announcement.patch_id;

        // Check for duplicate
        {
            let seen = self.seen_messages.read().await;
            if seen.contains(&patch_id) {
                return Err(GossipError::Duplicate(patch_id));
            }
        }

        // Verify signature
        {
            let verifier = self.verifier.read().await;
            verifier.verify_announcement(&announcement)?;
        }

        // Mark as seen
        self.seen_messages.write().await.insert(patch_id);

        // Store the patch
        let state = PatchState {
            announcement: announcement.clone(),
            status: PatchStatus::Pending,
            consensus: ConsensusState::new(patch_id),
            content: None,
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
            && let Some(content) = &state.content {
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

        let known_patches = patches.values().map(|state| PatchDigestEntry {
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
            .iter()
            .filter(|p| exclude.is_none_or(|e| *p != e))
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
