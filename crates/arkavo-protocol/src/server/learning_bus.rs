//! Learning bus for cross-agent lesson propagation
//!
//! Central bus connecting event capture, learning, and gossip protocol.

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use arkavo_gossip::{
    GossipConfig, GossipMessage, GossipProtocol, KeyRegistry, LessonAnnouncement, LessonDigest,
    sign_lesson_announcement,
};
use arkavo_router::learning::{AgentContribution, Episode, LearningModule, Lesson, ToolCallFormat};
use arkavo_router::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast, mpsc};
use uuid::Uuid;

use super::episode_buffer::{EpisodeBuffer, ToolObservation};
use super::policy_cache::PolicyCache;
use super::synthesis;
use super::tool_pattern_observer::ToolPatternObserver;

/// Configuration for learning thresholds and channel capacities
#[derive(Debug, Clone)]
pub struct LearningConfig {
    /// Observations needed before synthesizing an episode
    pub observation_threshold: usize,
    /// Episodes needed before synthesizing a lesson
    pub episode_threshold: usize,
    /// Minimum confidence required to accept a synthesized lesson
    pub min_lesson_confidence: f64,
    /// Capacity of broadcast channel for gossip out
    pub gossip_broadcast_capacity: usize,
    /// Capacity of event channel
    pub event_channel_capacity: usize,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            observation_threshold: 3,
            episode_threshold: 3,
            min_lesson_confidence: 0.5,
            gossip_broadcast_capacity: 256,
            event_channel_capacity: 1000,
        }
    }
}

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

/// Behavior advice based on learned lessons
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BehaviorAdvice {
    /// Use default behavior
    Default,
    /// Slow down due to learned lesson
    SlowDown {
        reason: String,
        lesson_id: Uuid,
        confidence: f64,
    },
    /// Avoid the sector entirely
    AvoidSector {
        reason: String,
        lesson_id: Uuid,
        confidence: f64,
    },
}

/// Central bus connecting event capture to learning and gossip
pub struct LearningBus {
    agent_id: String,
    swarm_id: String,
    /// Learning configuration
    config: LearningConfig,
    /// Channel for incoming learning events
    event_tx: mpsc::Sender<LearningEvent>,
    /// Event receiver (taken by event processing loop)
    event_rx: Arc<RwLock<Option<mpsc::Receiver<LearningEvent>>>>,
    /// Gossip protocol handler
    gossip: Arc<RwLock<GossipProtocol>>,
    /// Learning module for Thompson Sampling updates
    learning: Arc<RwLock<LearningModule>>,
    /// Agent keypair for signing messages
    keypair: Arc<AgentKeypair>,
    /// Channel for outgoing gossip messages (peer_id, message)
    gossip_out_tx: broadcast::Sender<(String, GossipMessage)>,
    /// Peer addresses for RPC calls (peer_id -> address)
    peer_addresses: Arc<RwLock<HashMap<String, String>>>,
    /// Cache of learned lessons for behavior policy checks
    policy_cache: Arc<RwLock<PolicyCache>>,
    /// Buffer for accumulating observations and episodes
    episode_buffer: Arc<RwLock<EpisodeBuffer>>,
    /// Router for LLM calls during synthesis (interior mutability for Arc usage)
    router: Arc<RwLock<Option<Arc<Router>>>>,
    /// Observer for capturing tool call patterns
    tool_pattern_observer: Arc<RwLock<ToolPatternObserver>>,
}

impl LearningBus {
    /// Create a new learning bus with default configuration
    pub fn new(
        agent_id: String,
        swarm_id: String,
        keypair: Arc<AgentKeypair>,
        gossip_config: GossipConfig,
    ) -> Self {
        Self::with_config(agent_id, swarm_id, keypair, gossip_config, LearningConfig::default())
    }

    /// Create a new learning bus with custom configuration
    pub fn with_config(
        agent_id: String,
        swarm_id: String,
        keypair: Arc<AgentKeypair>,
        gossip_config: GossipConfig,
        learning_config: LearningConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(learning_config.event_channel_capacity);
        let (gossip_out_tx, _) = broadcast::channel(learning_config.gossip_broadcast_capacity);

        // Create broadcast channel for lesson approvals
        let (lesson_approved_tx, _lesson_approved_rx) = broadcast::channel(64);

        let mut key_registry = KeyRegistry::new();
        #[allow(clippy::redundant_clone)]
        key_registry.register(agent_id.clone(), keypair.public_key().clone());

        let mut gossip_protocol =
            GossipProtocol::new(agent_id.clone(), gossip_config, key_registry);

        // Set up lesson approval callback
        gossip_protocol.set_lesson_approved_callback(lesson_approved_tx);

        let gossip = Arc::new(RwLock::new(gossip_protocol));
        let learning = Arc::new(RwLock::new(LearningModule::new()));

        Self {
            agent_id,
            swarm_id,
            config: learning_config.clone(),
            event_tx,
            event_rx: Arc::new(RwLock::new(Some(event_rx))),
            gossip,
            learning,
            keypair,
            gossip_out_tx,
            peer_addresses: Arc::new(RwLock::new(HashMap::new())),
            policy_cache: Arc::new(RwLock::new(PolicyCache::new())),
            episode_buffer: Arc::new(RwLock::new(EpisodeBuffer::with_thresholds(
                learning_config.observation_threshold,
                learning_config.episode_threshold,
            ))),
            router: Arc::new(RwLock::new(None)),
            tool_pattern_observer: Arc::new(RwLock::new(ToolPatternObserver::new(
                "unknown".to_string(),
            ))),
        }
    }

    /// Set the router for LLM-based synthesis
    pub async fn set_router(&self, router: Arc<Router>) {
        *self.router.write().await = Some(router);
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
    pub async fn add_peer(&self, peer_id: String, public_key: AgentPublicKey) {
        let gossip = self.gossip.write().await;
        gossip.add_peer(peer_id.clone()).await;
        gossip.register_key(peer_id, public_key).await;
    }

    /// Add a peer to gossip protocol (local mDNS discovery, key exchange deferred)
    pub async fn add_peer_discovered(&self, peer_id: String, address: Option<String>) {
        tracing::info!(
            "Adding discovered peer to gossip: {} (addr: {:?})",
            peer_id,
            address
        );
        let gossip = self.gossip.write().await;
        gossip.add_peer(peer_id.clone()).await;
        drop(gossip);

        // Store address if provided
        if let Some(addr) = address {
            self.peer_addresses.write().await.insert(peer_id, addr);
        }
    }

    /// Remove a peer from gossip protocol
    pub async fn remove_peer(&self, peer_id: &str) {
        tracing::info!("Removing peer from gossip: {}", peer_id);
        let gossip = self.gossip.write().await;
        gossip.remove_peer(peer_id).await;
        drop(gossip);

        // Remove address
        self.peer_addresses.write().await.remove(peer_id);
    }

    /// Get address for a peer
    pub async fn get_peer_address(&self, peer_id: &str) -> Option<String> {
        self.peer_addresses.read().await.get(peer_id).cloned()
    }

    /// Get all peer addresses
    pub async fn get_all_peer_addresses(&self) -> HashMap<String, String> {
        self.peer_addresses.read().await.clone()
    }

    /// Register a peer's public key for signature verification
    pub async fn register_peer_key(&self, peer_id: String, public_key: AgentPublicKey) {
        let gossip = self.gossip.write().await;
        gossip.register_key(peer_id.clone(), public_key).await;
        tracing::info!("Registered public key for peer: {}", peer_id);
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
            let _ = self
                .gossip_out_tx
                .send((peer_id.clone(), GossipMessage::AntiEntropy(digest.clone())));
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
        if stats.is_empty() {
            return Ok(());
        }

        tracing::debug!(
            "Learning stats available for {} agents (lesson synthesis pending)",
            stats.len()
        );

        Ok(())
    }

    /// Announce a lesson to the gossip network (signs before sending)
    pub async fn announce_lesson(
        &self,
        mut announcement: LessonAnnouncement,
    ) -> Result<(), String> {
        // Sign the announcement with our keypair
        sign_lesson_announcement(&mut announcement, &self.keypair)
            .map_err(|e| format!("Failed to sign lesson: {e}"))?;

        let gossip = self.gossip.read().await;
        let peers = gossip.select_propagation_peers(None).await;
        drop(gossip);

        let peer_count = peers.len();
        for peer_id in peers {
            let _ = self
                .gossip_out_tx
                .send((peer_id, GossipMessage::LessonAnnounce(announcement.clone())));
        }

        tracing::debug!(
            "Announced signed lesson {} to {} peers",
            announcement.lesson_id,
            peer_count
        );
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

    /// Check behavior policy for a sector based on learned lessons
    pub async fn check_behavior_policy(&self, sector_id: &str) -> BehaviorAdvice {
        let cache = self.policy_cache.read().await;

        // Check for avoid lessons first (highest priority)
        if let Some(lesson) = cache.should_avoid(sector_id) {
            return BehaviorAdvice::AvoidSector {
                reason: lesson.pattern.condition.clone(),
                lesson_id: lesson.id,
                confidence: lesson.confidence,
            };
        }

        // Check for slowdown lessons
        if let Some(lesson) = cache.should_slowdown(sector_id) {
            return BehaviorAdvice::SlowDown {
                reason: lesson.pattern.condition.clone(),
                lesson_id: lesson.id,
                confidence: lesson.confidence,
            };
        }

        BehaviorAdvice::Default
    }

    /// Get the policy cache reference
    pub fn policy_cache(&self) -> &Arc<RwLock<PolicyCache>> {
        &self.policy_cache
    }

    /// Subscribe to lesson approval notifications
    pub async fn subscribe_lesson_approvals(
        &self,
    ) -> Option<broadcast::Receiver<LessonAnnouncement>> {
        self.gossip.read().await.subscribe_lesson_approvals()
    }

    /// Add a lesson to the policy cache
    pub async fn add_lesson_to_cache(&self, lesson: Lesson) {
        let mut cache = self.policy_cache.write().await;
        cache.add_lesson(lesson);
    }

    /// Get count of cached lessons
    pub async fn cached_lesson_count(&self) -> usize {
        self.policy_cache.read().await.len()
    }

    /// Get the episode buffer reference
    pub fn episode_buffer(&self) -> &Arc<RwLock<EpisodeBuffer>> {
        &self.episode_buffer
    }

    /// Take the event receiver for the event processing loop (can only be called once)
    pub async fn take_event_receiver(&self) -> Option<mpsc::Receiver<LearningEvent>> {
        self.event_rx.write().await.take()
    }

    /// Synthesize an episode from observations using LLM
    pub async fn synthesize_episode(
        &self,
        observations: &[ToolObservation],
        category: &str,
    ) -> Result<Episode, String> {
        let router_guard = self.router.read().await;
        let router = router_guard.as_ref().ok_or("Router not configured")?;
        synthesis::synthesize_episode(
            router,
            &self.agent_id,
            &self.swarm_id,
            observations,
            category,
        )
        .await
    }

    /// Synthesize a lesson from episodes using LLM
    pub async fn synthesize_lesson(
        &self,
        episodes: &[Episode],
        category: &str,
    ) -> Result<Option<Lesson>, String> {
        let router_guard = self.router.read().await;
        let router = router_guard.as_ref().ok_or("Router not configured")?;
        synthesis::synthesize_lesson(
            router,
            &self.agent_id,
            &self.swarm_id,
            episodes,
            category,
            self.config.min_lesson_confidence,
        )
        .await
    }

    /// Get the learning configuration
    pub fn config(&self) -> &LearningConfig {
        &self.config
    }

    /// Get the tool pattern observer reference
    pub fn tool_pattern_observer(&self) -> &Arc<RwLock<ToolPatternObserver>> {
        &self.tool_pattern_observer
    }

    /// Record a successful tool call pattern
    pub async fn record_tool_pattern_success(
        &self,
        tool_name: &str,
        format: ToolCallFormat,
        raw_invocation: &str,
        args: &serde_json::Value,
    ) {
        let mut observer = self.tool_pattern_observer.write().await;
        observer.record_success(tool_name, format, raw_invocation, args);
    }

    /// Add a tool pattern lesson to the policy cache
    pub async fn add_tool_pattern_to_cache(&self, lesson: Lesson) {
        let mut cache = self.policy_cache.write().await;
        cache.add_lesson(lesson);
    }

    /// Get few-shot examples for prompt injection from policy cache
    pub async fn get_few_shot_examples(
        &self,
        tool_names: &[String],
        _format: ToolCallFormat,
    ) -> String {
        let cache = self.policy_cache.read().await;
        cache.get_few_shot_examples(tool_names)
    }

    /// Update the model name for pattern attribution
    pub async fn set_model_name(&self, model_name: String) {
        let mut observer = self.tool_pattern_observer.write().await;
        observer.set_model_name(model_name);
    }

    /// Check if there are patterns ready for lesson synthesis
    pub async fn has_ready_patterns(&self) -> bool {
        let observer = self.tool_pattern_observer.read().await;
        observer.has_ready_patterns()
    }

    /// Get the number of cached tool format lessons
    pub async fn cached_tool_pattern_count(&self) -> usize {
        self.policy_cache.read().await.tool_format_lesson_count()
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
