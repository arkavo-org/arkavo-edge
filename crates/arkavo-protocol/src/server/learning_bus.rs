//! Learning bus for cross-agent lesson propagation
//!
//! Central bus connecting event capture, learning, and gossip protocol.

use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_gossip::{
    GossipConfig, GossipMessage, GossipProtocol, KeyRegistry, LessonAnnouncement, LessonDigest,
};
use arkavo_router::learning::{AgentContribution, LearningModule};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, RwLock};
use uuid::Uuid;

/// Learning event types that flow through the bus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LearningEvent {
    /// Tool was called with result
    ToolCall {
        tool_name: String,
        args: serde_json::Value,
        result: String,
        success: bool,
        latency_ms: u64,
    },
    /// Task completed
    TaskComplete {
        task_id: Uuid,
        category: String,
        success: bool,
        agent_contributions: Vec<AgentContribution>,
    },
    /// Gossip message received from peer
    GossipReceived(GossipMessage),
}

/// Central bus connecting event capture to learning and gossip
pub struct LearningBus {
    agent_id: String,
    swarm_id: String,
    /// Channel for incoming learning events
    event_tx: mpsc::Sender<LearningEvent>,
    /// Event receiver (will be used for event processing loop)
    #[allow(dead_code)]
    event_rx: Arc<RwLock<mpsc::Receiver<LearningEvent>>>,
    /// Gossip protocol handler
    gossip: Arc<RwLock<GossipProtocol>>,
    /// Learning module for Thompson Sampling updates
    learning: Arc<RwLock<LearningModule>>,
    /// Agent keypair for signing messages
    keypair: Arc<AgentKeypair>,
    /// Channel for outgoing gossip messages (peer_id, message)
    gossip_out_tx: broadcast::Sender<(String, GossipMessage)>,
}

impl LearningBus {
    /// Create a new learning bus
    pub fn new(
        agent_id: String,
        swarm_id: String,
        keypair: Arc<AgentKeypair>,
        gossip_config: GossipConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(1000);
        let (gossip_out_tx, _) = broadcast::channel(256);

        let mut key_registry = KeyRegistry::new();
        #[allow(clippy::redundant_clone)]
        key_registry.register(agent_id.clone(), keypair.public_key().clone());

        let gossip = Arc::new(RwLock::new(GossipProtocol::new(
            agent_id.clone(),
            gossip_config,
            key_registry,
        )));

        let learning = Arc::new(RwLock::new(LearningModule::new()));

        Self {
            agent_id,
            swarm_id,
            event_tx,
            event_rx: Arc::new(RwLock::new(event_rx)),
            gossip,
            learning,
            keypair,
            gossip_out_tx,
        }
    }

    /// Get agent ID
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Get swarm ID
    pub fn swarm_id(&self) -> &str {
        &self.swarm_id
    }

    /// Get sender for submitting learning events
    pub fn sender(&self) -> mpsc::Sender<LearningEvent> {
        self.event_tx.clone()
    }

    /// Subscribe to outgoing gossip messages for transport
    pub fn subscribe_gossip_out(&self) -> broadcast::Receiver<(String, GossipMessage)> {
        self.gossip_out_tx.subscribe()
    }

    /// Handle incoming gossip message from peer
    pub async fn handle_gossip(&self, message: GossipMessage) -> Vec<GossipMessage> {
        let gossip = self.gossip.read().await;
        match gossip.handle_message(message).await {
            Ok(responses) => responses,
            Err(e) => {
                tracing::warn!("Gossip message error: {}", e);
                vec![]
            }
        }
    }

    /// Add a peer to gossip protocol with their public key
    pub async fn add_peer(&self, peer_id: String, public_key: arkavo_crypto::AgentPublicKey) {
        let gossip = self.gossip.write().await;
        gossip.add_peer(peer_id.clone()).await;
        gossip.register_key(peer_id, public_key).await;
    }

    /// Add a peer to gossip protocol (local mDNS discovery, key exchange deferred)
    pub async fn add_peer_discovered(&self, peer_id: String) {
        tracing::info!("Adding discovered peer to gossip: {}", peer_id);
        let gossip = self.gossip.write().await;
        gossip.add_peer(peer_id).await;
    }

    /// Remove a peer from gossip protocol
    pub async fn remove_peer(&self, peer_id: &str) {
        tracing::info!("Removing peer from gossip: {}", peer_id);
        let gossip = self.gossip.write().await;
        gossip.remove_peer(peer_id).await;
    }

    /// Get the number of connected peers
    pub async fn peer_count(&self) -> usize {
        self.gossip.read().await.peer_count().await
    }

    /// Run anti-entropy synchronization with peers
    pub async fn run_anti_entropy(&self) -> Result<(), String> {
        let gossip = self.gossip.read().await;
        let digest = gossip.create_digest().await;
        let lesson_digest = gossip.create_lesson_digest().await;
        drop(gossip);

        // Select peers to send digests to
        let gossip = self.gossip.read().await;
        let peers = gossip.select_propagation_peers(None).await;
        drop(gossip);

        // Send patch digest to selected peers
        for peer_id in &peers {
            let _ = self.gossip_out_tx.send((
                peer_id.clone(),
                GossipMessage::AntiEntropy(digest.clone()),
            ));
            let _ = self.gossip_out_tx.send((
                peer_id.clone(),
                GossipMessage::LessonDigest(lesson_digest.clone()),
            ));
        }

        tracing::debug!(
            "Anti-entropy sent to {} peers: {} patches, {} lessons",
            peers.len(),
            digest.known_patches.len(),
            lesson_digest.known_lessons.len()
        );

        Ok(())
    }

    /// Synthesize and propagate lessons from accumulated learning
    pub async fn synthesize_and_propagate_lessons(&self) -> Result<(), String> {
        // Get learning stats for all agents
        let learning = self.learning.read().await;
        let stats = learning.get_all_stats().await;
        drop(learning);

        // For now, we only propagate if we have significant learning
        // Future: implement lesson synthesis from episodes
        if stats.is_empty() {
            return Ok(());
        }

        tracing::debug!(
            "Learning stats available for {} agents (lesson synthesis pending)",
            stats.len()
        );

        Ok(())
    }

    /// Announce a lesson to the gossip network
    pub async fn announce_lesson(&self, announcement: LessonAnnouncement) -> Result<(), String> {
        let gossip = self.gossip.read().await;
        let peers = gossip.select_propagation_peers(None).await;
        drop(gossip);

        for peer_id in peers {
            let _ = self.gossip_out_tx.send((
                peer_id,
                GossipMessage::LessonAnnounce(announcement.clone()),
            ));
        }

        Ok(())
    }

    /// Get gossip protocol reference for direct access
    pub fn gossip(&self) -> &Arc<RwLock<GossipProtocol>> {
        &self.gossip
    }

    /// Get learning module reference for direct access
    pub fn learning(&self) -> &Arc<RwLock<LearningModule>> {
        &self.learning
    }

    /// Get keypair reference
    pub fn keypair(&self) -> &Arc<AgentKeypair> {
        &self.keypair
    }

    /// Create a lesson digest for anti-entropy
    pub async fn create_lesson_digest(&self) -> LessonDigest {
        self.gossip.read().await.create_lesson_digest().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_learning_bus_creation() {
        let keypair = Arc::new(AgentKeypair::generate());
        let bus = LearningBus::new(
            "test-agent".to_string(),
            "test-swarm".to_string(),
            keypair,
            GossipConfig::default(),
        );

        assert_eq!(bus.agent_id(), "test-agent");
        assert_eq!(bus.swarm_id(), "test-swarm");
        assert_eq!(bus.peer_count().await, 0);
    }

    #[tokio::test]
    async fn test_peer_management() {
        let keypair = Arc::new(AgentKeypair::generate());
        let bus = LearningBus::new(
            "test-agent".to_string(),
            "test-swarm".to_string(),
            keypair.clone(),
            GossipConfig::default(),
        );

        bus.add_peer("peer-1".to_string(), keypair.public_key().clone())
            .await;
        assert_eq!(bus.peer_count().await, 1);

        bus.remove_peer("peer-1").await;
        assert_eq!(bus.peer_count().await, 0);
    }
}
