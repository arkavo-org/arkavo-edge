//! Learning bus for cross-agent lesson propagation
//!
//! Central bus connecting event capture, learning, and gossip protocol.

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_gossip::{GossipConfig, GossipMessage, GossipProtocol, KeyRegistry};
use arkavo_router::Router;
use arkavo_router::learning::{AgentContribution, LearningModule, LearningStore};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast, mpsc};
use uuid::Uuid;

use super::episode_buffer::EpisodeBuffer;
use super::policy_cache::PolicyCache;
use super::tool_pattern_observer::ToolPatternObserver;

use arkavo_memory::case_retrieval::CaseIndex;
use arkavo_memory::embeddings::EmbeddingService;

/// Minimum success rate required before broadcasting an advisor adjustment to peers
pub(super) const BROADCAST_MIN_SUCCESS_RATE: f64 = 0.7;
/// Minimum feedback count before broadcasting an advisor adjustment to peers
pub(super) const BROADCAST_MIN_FEEDBACK_COUNT: u32 = 5;
/// Minimum applications before broadcasting an advisor adjustment to peers
pub(super) const BROADCAST_MIN_APPLICATIONS: u32 = 3;

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
        /// Links to the routing decision
        #[serde(default)]
        decision_trace_id: Option<Uuid>,
        /// Position in multi-step execution
        #[serde(default)]
        step_index: u16,
        /// Which model generated this call
        #[serde(default)]
        model_name: Option<String>,
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
    /// Human correction of a recent action
    HumanCorrection {
        text: String,
        /// DecisionTrace ID of the action being corrected
        trace_id: Option<Uuid>,
        /// Model that produced the corrected action
        model_name: Option<String>,
    },
    /// Human reinforcement of a recent action
    HumanReinforcement {
        text: String,
        /// DecisionTrace ID of the action being reinforced
        trace_id: Option<Uuid>,
    },
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
    pub(super) agent_id: String,
    pub(super) swarm_id: String,
    /// Learning configuration
    pub(super) config: LearningConfig,
    /// Channel for incoming learning events
    event_tx: mpsc::Sender<LearningEvent>,
    /// Event receiver (taken by event processing loop)
    event_rx: Arc<RwLock<Option<mpsc::Receiver<LearningEvent>>>>,
    /// Gossip protocol handler
    pub(super) gossip: Arc<RwLock<GossipProtocol>>,
    /// Learning module for Thompson Sampling updates
    pub(super) learning: Arc<RwLock<LearningModule>>,
    /// Agent keypair for signing messages
    pub(super) keypair: Arc<AgentKeypair>,
    /// Channel for outgoing gossip messages (peer_id, message)
    pub(super) gossip_out_tx: broadcast::Sender<(String, GossipMessage)>,
    /// Peer addresses for RPC calls (peer_id -> address)
    pub(super) peer_addresses: Arc<RwLock<HashMap<String, String>>>,
    /// Cache of learned lessons for behavior policy checks
    pub(super) policy_cache: Arc<RwLock<PolicyCache>>,
    /// Buffer for accumulating observations and episodes
    episode_buffer: Arc<RwLock<EpisodeBuffer>>,
    /// Router for LLM calls during synthesis (interior mutability for Arc usage)
    pub(super) router: Arc<RwLock<Option<Arc<Router>>>>,
    /// Observer for capturing tool call patterns
    pub(super) tool_pattern_observer: Arc<RwLock<ToolPatternObserver>>,
    /// Case-based retrieval index for episodes
    pub(super) case_index: Arc<CaseIndex>,
    /// SQLite-backed persistent store for lessons and episodes
    pub(super) learning_store: Arc<RwLock<Option<Arc<LearningStore>>>>,
}

impl LearningBus {
    /// Create a new learning bus with default configuration
    pub fn new(
        agent_id: String,
        swarm_id: String,
        keypair: Arc<AgentKeypair>,
        gossip_config: GossipConfig,
    ) -> Self {
        Self::with_config(
            agent_id,
            swarm_id,
            keypair,
            gossip_config,
            LearningConfig::default(),
        )
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
        let embedding_service = Arc::new(EmbeddingService::new());
        let case_index = Arc::new(CaseIndex::new(embedding_service));

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
            case_index,
            learning_store: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the persistent learning store and load existing lessons
    pub async fn init_persistence(&self, db_path: &std::path::Path) {
        match LearningStore::new(db_path).await {
            Ok(store) => {
                let store = Arc::new(store);
                // Load existing lessons into policy cache
                if let Ok(lessons) = store.get_lessons(&self.swarm_id).await
                    && !lessons.is_empty()
                {
                    let mut cache = self.policy_cache.write().await;
                    for lesson in &lessons {
                        cache.add_lesson(lesson.clone());
                    }
                    tracing::info!(
                        count = lessons.len(),
                        "Loaded persisted lessons into policy cache"
                    );
                }
                *self.learning_store.write().await = Some(store);
                tracing::info!("Learning store initialized at {}", db_path.display());
            }
            Err(e) => {
                tracing::warn!("Learning persistence unavailable: {e}");
            }
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

    /// Get the policy cache reference
    pub fn policy_cache(&self) -> &Arc<RwLock<PolicyCache>> {
        &self.policy_cache
    }

    /// Get the episode buffer reference
    pub fn episode_buffer(&self) -> &Arc<RwLock<EpisodeBuffer>> {
        &self.episode_buffer
    }

    /// Take the event receiver for the event processing loop (can only be called once)
    pub async fn take_event_receiver(&self) -> Option<mpsc::Receiver<LearningEvent>> {
        self.event_rx.write().await.take()
    }

    /// Get the learning configuration
    pub fn config(&self) -> &LearningConfig {
        &self.config
    }

    /// Get the router reference for advisor access
    pub fn router(&self) -> &Arc<RwLock<Option<Arc<Router>>>> {
        &self.router
    }

    /// Get the gossip outbound channel sender
    pub fn gossip_out_tx(&self) -> &broadcast::Sender<(String, GossipMessage)> {
        &self.gossip_out_tx
    }

    /// Get the case-based retrieval index
    pub fn case_index(&self) -> &Arc<CaseIndex> {
        &self.case_index
    }

    /// Add a peer to gossip protocol with their public key
    pub async fn add_peer(&self, peer_id: String, public_key: arkavo_crypto::AgentPublicKey) {
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
    pub async fn register_peer_key(
        &self,
        peer_id: String,
        public_key: arkavo_crypto::AgentPublicKey,
    ) {
        let gossip = self.gossip.write().await;
        gossip.register_key(peer_id.clone(), public_key).await;
        tracing::info!("Registered public key for peer: {}", peer_id);
    }

    /// Get the number of connected peers
    pub async fn peer_count(&self) -> usize {
        self.gossip.read().await.peer_count().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_router::learning::{Lesson, LessonPattern};

    fn make_bus() -> LearningBus {
        let keypair = Arc::new(AgentKeypair::generate());
        LearningBus::new(
            "test-agent".to_string(),
            "test-swarm".to_string(),
            keypair,
            GossipConfig::default(),
        )
    }

    #[tokio::test]
    async fn test_learning_bus_creation() {
        let bus = make_bus();

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

    #[tokio::test]
    async fn test_behavior_guidance_via_bus() {
        let bus = make_bus();

        let lesson = Lesson::new(
            "agent-1".to_string(),
            "local".to_string(),
            "code".to_string(),
            LessonPattern::new(
                "Agent returns generic non-answers".to_string(),
                "Reduce weight".to_string(),
                "Substantive response".to_string(),
            ),
            0.8,
            1,
        );
        bus.add_lesson_to_cache(lesson).await;

        let guidance = bus.get_behavior_guidance(None).await;
        assert!(!guidance.is_empty());
        assert!(guidance.contains("generic non-answers"));
    }

    #[tokio::test]
    async fn test_record_quality_via_bus() {
        let bus = make_bus();

        bus.record_quality("a", "code", 0.5).await;

        let trends = bus.get_quality_trends().await;
        assert_eq!(trends.len(), 1);
        assert_eq!(trends[0].scores, vec![0.5]);
    }

    #[tokio::test]
    async fn test_behavior_lesson_count_via_bus() {
        let bus = make_bus();

        for i in 0..3 {
            let lesson = Lesson::new(
                "a".to_string(),
                "s".to_string(),
                "code".to_string(),
                LessonPattern::new(
                    format!("issue-{i}"),
                    "action".to_string(),
                    "outcome".to_string(),
                ),
                0.8,
                1,
            );
            bus.add_lesson_to_cache(lesson).await;
        }

        assert_eq!(bus.behavior_lesson_count().await, 3);
    }

    #[tokio::test]
    async fn test_lesson_receiver_stores_in_policy_cache() {
        let bus = make_bus();
        let (tx, rx) = mpsc::channel(16);

        bus.start_lesson_receiver(rx);

        let lesson = Lesson::new(
            "agent-x".to_string(),
            "test-swarm".to_string(),
            "security".to_string(),
            LessonPattern::new(
                "Agent agent-x returns empty responses".to_string(),
                "Avoid routing".to_string(),
                "Non-empty response".to_string(),
            ),
            0.9,
            1,
        );
        tx.send(lesson).await.expect("send should succeed");

        // Yield briefly to let the spawned task process the lesson
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(
            bus.cached_lesson_count().await >= 1,
            "lesson should be stored in policy cache"
        );
        let guidance = bus.get_behavior_guidance(None).await;
        assert!(
            guidance.contains("empty responses"),
            "guidance should contain lesson condition text"
        );
    }
}
