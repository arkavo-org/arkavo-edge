//! Network bridge connecting gossip protocol to transport
//!
//! Bridges the gossip protocol to async message channels for
//! patch propagation and vote coordination.

use std::sync::Arc;

use tokio::sync::mpsc;
use uuid::Uuid;

use arkavo_crypto::AgentKeypair;
use arkavo_gossip::{
    GossipConfig, GossipMessage, GossipProtocol, KeyRegistry, PatchAnnouncement, PatchDelivery,
    PatchStatus, PatchVote, compute_content_hash, sign_announcement, sign_vote,
};

use crate::error::{AutoLearnError, AutoLearnResult};
use crate::patchlet::Patchlet;

/// Configuration for the network bridge
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Gossip protocol configuration
    pub gossip: GossipConfig,
    /// Channel buffer size for outgoing messages
    pub outbox_buffer: usize,
    /// Channel buffer size for incoming messages
    pub inbox_buffer: usize,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            gossip: GossipConfig::default(),
            outbox_buffer: 100,
            inbox_buffer: 100,
        }
    }
}

/// Incoming patch delivery from the network
#[derive(Debug, Clone)]
pub struct IncomingPatch {
    /// The patch ID
    pub patch_id: Uuid,
    /// The patchlet content
    pub patchlet: Patchlet,
    /// Votes received with the patch
    pub votes: Vec<PatchVote>,
}

/// Bridge between gossip protocol and network transport
pub struct GossipNetworkBridge {
    /// The gossip protocol handler
    protocol: GossipProtocol,
    /// Our keypair for signing
    keypair: AgentKeypair,
    /// Channel sender for outgoing messages
    outbox_tx: mpsc::Sender<GossipMessage>,
    /// Channel receiver for outgoing messages (for transport to consume)
    outbox_rx: Option<mpsc::Receiver<GossipMessage>>,
    /// Channel sender for incoming patches (after processing)
    patch_tx: mpsc::Sender<IncomingPatch>,
    /// Channel receiver for incoming patches (for learner to consume)
    patch_rx: Option<mpsc::Receiver<IncomingPatch>>,
}

impl GossipNetworkBridge {
    /// Create a new network bridge
    pub fn new(agent_id: String, keypair: AgentKeypair, config: NetworkConfig) -> Self {
        let mut key_registry = KeyRegistry::new();
        key_registry.register(agent_id.clone(), keypair.public_key().clone());

        let protocol = GossipProtocol::new(agent_id, config.gossip.clone(), key_registry);

        let (outbox_tx, outbox_rx) = mpsc::channel(config.outbox_buffer);
        let (patch_tx, patch_rx) = mpsc::channel(config.inbox_buffer);

        Self {
            protocol,
            keypair,
            outbox_tx,
            outbox_rx: Some(outbox_rx),
            patch_tx,
            patch_rx: Some(patch_rx),
        }
    }

    /// Take the outbox receiver for the transport layer to consume
    ///
    /// Returns None if already taken.
    pub fn take_outbox(&mut self) -> Option<mpsc::Receiver<GossipMessage>> {
        self.outbox_rx.take()
    }

    /// Take the patch receiver for the learner to consume
    ///
    /// Returns None if already taken.
    pub fn take_patch_receiver(&mut self) -> Option<mpsc::Receiver<IncomingPatch>> {
        self.patch_rx.take()
    }

    /// Add a peer to the gossip network
    pub async fn add_peer(&self, peer_id: String) {
        self.protocol.add_peer(peer_id).await;
    }

    /// Remove a peer from the gossip network
    pub async fn remove_peer(&self, peer_id: &str) {
        self.protocol.remove_peer(peer_id).await;
    }

    /// Register a peer's public key for verification
    pub async fn register_peer_key(&self, peer_id: String, pubkey: arkavo_crypto::AgentPublicKey) {
        self.protocol.register_key(peer_id, pubkey).await;
    }

    /// Get the number of known peers
    pub async fn peer_count(&self) -> usize {
        self.protocol.peer_count().await
    }

    /// Broadcast a patchlet to the network
    pub async fn broadcast_patch(&self, patchlet: &Patchlet) -> AutoLearnResult<Uuid> {
        let patch_id = Uuid::new_v4();

        // Serialize and hash the patchlet
        let content = patchlet.to_bytes()?;
        let content_hash = compute_content_hash(&content);

        // Create and sign announcement
        let mut announcement =
            PatchAnnouncement::new(patch_id, content_hash, self.agent_id().to_string(), vec![]);
        sign_announcement(&mut announcement, &self.keypair)
            .map_err(|e| AutoLearnError::Network(e.to_string()))?;

        // Send announcement
        self.outbox_tx
            .send(GossipMessage::PatchAnnounce(announcement))
            .await
            .map_err(|e| AutoLearnError::Network(e.to_string()))?;

        // Send delivery with content
        let delivery = PatchDelivery {
            patch_id,
            content,
            content_hash,
            votes: vec![],
        };
        self.outbox_tx
            .send(GossipMessage::PatchDelivery(delivery))
            .await
            .map_err(|e| AutoLearnError::Network(e.to_string()))?;

        Ok(patch_id)
    }

    /// Handle an incoming gossip message
    pub async fn handle_incoming(&self, message: GossipMessage) -> AutoLearnResult<()> {
        // Process through protocol
        let responses = self
            .protocol
            .handle_message(message.clone())
            .await
            .map_err(|e| {
                if matches!(e, arkavo_gossip::GossipError::Duplicate(_)) {
                    AutoLearnError::Internal("Duplicate message".to_string())
                } else {
                    AutoLearnError::Gossip(e)
                }
            })?;

        // If it's a delivery, extract and forward the patchlet
        if let GossipMessage::PatchDelivery(delivery) = message {
            let patchlet = Patchlet::from_bytes(&delivery.content)?;
            let incoming = IncomingPatch {
                patch_id: delivery.patch_id,
                patchlet,
                votes: delivery.votes,
            };

            self.patch_tx
                .send(incoming)
                .await
                .map_err(|e| AutoLearnError::Network(format!("Failed to forward patch: {}", e)))?;
        }

        // Forward response messages to outbox
        for response in responses {
            self.outbox_tx
                .send(response)
                .await
                .map_err(|e| AutoLearnError::Network(format!("Failed to send response: {}", e)))?;
        }

        Ok(())
    }

    /// Vote on a patch
    pub async fn vote_on_patch(&self, patch_id: Uuid, approve: bool) -> AutoLearnResult<()> {
        // Create and sign vote
        let mut vote = self
            .protocol
            .vote(patch_id, approve)
            .await
            .map_err(AutoLearnError::Gossip)?;

        sign_vote(&mut vote, &self.keypair).map_err(|e| AutoLearnError::Network(e.to_string()))?;

        // Send vote
        self.outbox_tx
            .send(GossipMessage::PatchVote(vote))
            .await
            .map_err(|e| AutoLearnError::Network(e.to_string()))?;

        Ok(())
    }

    /// Get the status of a patch
    pub async fn get_patch_status(&self, patch_id: Uuid) -> Option<PatchStatus> {
        self.protocol.get_patch_status(patch_id).await
    }

    /// Create an anti-entropy digest for synchronization
    pub async fn create_digest(&self) -> GossipMessage {
        GossipMessage::AntiEntropy(self.protocol.create_digest().await)
    }

    /// Get agent ID
    fn agent_id(&self) -> &str {
        // The protocol stores the agent_id but doesn't expose it,
        // so we derive it from the keypair public key hash
        // In practice, this would be stored separately
        "local-agent"
    }
}

/// Bridge handle for spawning the network processing task
pub struct NetworkHandle {
    /// Incoming message sender (for transport to send to)
    pub inbox_tx: mpsc::Sender<GossipMessage>,
    /// Outgoing message receiver (for transport to consume)
    pub outbox_rx: mpsc::Receiver<GossipMessage>,
    /// Patch receiver (for learner to consume)
    pub patch_rx: mpsc::Receiver<IncomingPatch>,
}

/// Create a network bridge with handles for integration
pub fn create_network(
    agent_id: String,
    keypair: AgentKeypair,
    config: NetworkConfig,
) -> (Arc<GossipNetworkBridge>, NetworkHandle) {
    let mut bridge = GossipNetworkBridge::new(agent_id, keypair, config);

    let outbox_rx = bridge.take_outbox().expect("outbox should be available");
    let patch_rx = bridge
        .take_patch_receiver()
        .expect("patch receiver should be available");

    // Create inbox channel for receiving messages
    let (inbox_tx, _inbox_rx) = mpsc::channel(100);

    let handle = NetworkHandle {
        inbox_tx,
        outbox_rx,
        patch_rx,
    };

    (Arc::new(bridge), handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patchlet::{PainTrigger, VerificationSummary};
    use arkavo_ensemble::GenerationMethod;
    use chrono::Utc;
    use torg_core::{Builder, Token};

    fn create_simple_graph() -> torg_core::Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.finish().unwrap()
    }

    fn create_test_patchlet() -> Patchlet {
        let graph = create_simple_graph();
        let trigger = PainTrigger {
            anomaly_id: Uuid::new_v4(),
            description: "Test".to_string(),
            severity: 0.5,
            timestamp: Utc::now(),
        };
        Patchlet::new(graph, trigger, GenerationMethod::Manual).with_verification(
            VerificationSummary {
                passed: true,
                invariant_checks: 1,
                sat_probes: 10,
            },
        )
    }

    #[tokio::test]
    async fn test_bridge_creation() {
        let keypair = AgentKeypair::generate();
        let bridge =
            GossipNetworkBridge::new("test-agent".to_string(), keypair, NetworkConfig::default());

        assert_eq!(bridge.peer_count().await, 0);
    }

    #[tokio::test]
    async fn test_add_remove_peers() {
        let keypair = AgentKeypair::generate();
        let bridge =
            GossipNetworkBridge::new("test-agent".to_string(), keypair, NetworkConfig::default());

        bridge.add_peer("peer-1".to_string()).await;
        bridge.add_peer("peer-2".to_string()).await;
        assert_eq!(bridge.peer_count().await, 2);

        bridge.remove_peer("peer-1").await;
        assert_eq!(bridge.peer_count().await, 1);
    }

    #[tokio::test]
    async fn test_broadcast_patch() {
        let keypair = AgentKeypair::generate();
        let mut bridge =
            GossipNetworkBridge::new("test-agent".to_string(), keypair, NetworkConfig::default());

        let mut outbox = bridge.take_outbox().unwrap();

        let patchlet = create_test_patchlet();
        let patch_id = bridge.broadcast_patch(&patchlet).await.unwrap();

        assert!(!patch_id.is_nil());

        // Should receive announcement and delivery
        let msg1 = outbox.recv().await.unwrap();
        assert!(matches!(msg1, GossipMessage::PatchAnnounce(_)));

        let msg2 = outbox.recv().await.unwrap();
        assert!(matches!(msg2, GossipMessage::PatchDelivery(_)));
    }

    #[tokio::test]
    async fn test_create_network() {
        let keypair = AgentKeypair::generate();
        let (bridge, _handle) =
            create_network("test-agent".to_string(), keypair, NetworkConfig::default());

        assert_eq!(bridge.peer_count().await, 0);
    }
}
