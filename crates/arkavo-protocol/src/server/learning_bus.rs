//! Learning bus for cross-agent lesson propagation
//!
//! Central bus connecting event capture, learning, and gossip protocol.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use arkavo_gossip::{
    GossipConfig, GossipMessage, GossipProtocol, KeyRegistry, LessonAnnouncement, LessonDigest,
    sign_lesson_announcement,
};
use arkavo_llm::Message;
use arkavo_router::Router;
use arkavo_router::learning::{
    AgentContribution, Episode, EpisodeOutcome, LearningModule, Lesson, LessonPattern, Observation,
    QualityMetrics,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast, mpsc};
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

/// Cache of lessons indexed by sector for fast policy lookup
pub struct PolicyCache {
    /// Lessons indexed by sector ID
    lessons_by_sector: HashMap<String, Vec<Lesson>>,
    /// All lessons by ID for quick lookup
    lessons_by_id: HashMap<Uuid, Lesson>,
}

impl Default for PolicyCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyCache {
    /// Create a new empty policy cache
    pub fn new() -> Self {
        Self {
            lessons_by_sector: HashMap::new(),
            lessons_by_id: HashMap::new(),
        }
    }

    /// Add a lesson to the cache
    pub fn add_lesson(&mut self, lesson: Lesson) {
        // Extract sector from condition (simple parsing)
        let sector_id = Self::extract_sector(&lesson.pattern.condition);

        if let Some(sector) = sector_id {
            self.lessons_by_sector
                .entry(sector)
                .or_default()
                .push(lesson.clone());
        }

        self.lessons_by_id.insert(lesson.id, lesson);
    }

    /// Check if there's a slowdown lesson for a sector
    pub fn should_slowdown(&self, sector_id: &str) -> Option<&Lesson> {
        self.lessons_by_sector.get(sector_id).and_then(|lessons| {
            lessons
                .iter()
                .find(|l| l.pattern.action.to_lowercase().contains("slow"))
        })
    }

    /// Check if there's an avoid lesson for a sector
    pub fn should_avoid(&self, sector_id: &str) -> Option<&Lesson> {
        self.lessons_by_sector.get(sector_id).and_then(|lessons| {
            lessons
                .iter()
                .find(|l| l.pattern.action.to_lowercase().contains("avoid"))
        })
    }

    /// Get a lesson by ID
    pub fn get_lesson(&self, lesson_id: &Uuid) -> Option<&Lesson> {
        self.lessons_by_id.get(lesson_id)
    }

    /// Get all lessons for a sector
    pub fn get_lessons_for_sector(&self, sector_id: &str) -> Vec<&Lesson> {
        self.lessons_by_sector
            .get(sector_id)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Get total lesson count
    pub fn len(&self) -> usize {
        self.lessons_by_id.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.lessons_by_id.is_empty()
    }

    /// Extract sector ID from a condition string
    /// Supports formats: "sector_4", "sector_id == 4", "IF sector_4"
    fn extract_sector(condition: &str) -> Option<String> {
        let lower = condition.to_lowercase();

        // Try pattern: sector_N or sector N
        if let Some(start) = lower.find("sector") {
            let rest = &condition[start + 6..];
            let rest = rest.trim_start_matches(['_', ' ', '=']);

            // Extract the number/identifier
            let sector: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();

            if !sector.is_empty() {
                return Some(sector);
            }
        }

        None
    }
}

/// Buffer for accumulating tool call observations before episode synthesis
pub struct EpisodeBuffer {
    /// Observations indexed by category
    observations: HashMap<String, VecDeque<ToolObservation>>,
    /// Episodes ready for lesson synthesis, indexed by category
    episodes: HashMap<String, Vec<Episode>>,
    /// Threshold for synthesizing an episode from observations
    observation_threshold: usize,
    /// Threshold for synthesizing a lesson from episodes
    episode_threshold: usize,
}

/// A single observation from a tool call
#[derive(Debug, Clone)]
pub struct ToolObservation {
    /// Tool name that was called
    pub tool_name: String,
    /// Arguments passed to the tool
    pub args: serde_json::Value,
    /// Result from the tool
    pub result: String,
    /// Whether the call succeeded
    pub success: bool,
    /// Latency in milliseconds
    pub latency_ms: u64,
    /// When this observation was recorded
    pub timestamp: chrono::DateTime<Utc>,
}

impl Default for EpisodeBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl EpisodeBuffer {
    /// Create a new episode buffer with default thresholds
    pub fn new() -> Self {
        Self {
            observations: HashMap::new(),
            episodes: HashMap::new(),
            observation_threshold: 3, // Synthesize episode after 3 observations
            episode_threshold: 3,     // Synthesize lesson after 3 episodes
        }
    }

    /// Add an observation to the buffer
    pub fn add_observation(&mut self, obs: ToolObservation) {
        // Infer category from tool name
        let category = Self::infer_category(&obs.tool_name);
        self.observations
            .entry(category)
            .or_default()
            .push_back(obs);
    }

    /// Check if any category has enough observations for episode synthesis
    pub fn ready_for_episode_synthesis(&self) -> Option<String> {
        for (category, obs) in &self.observations {
            if obs.len() >= self.observation_threshold {
                return Some(category.clone());
            }
            // Also trigger on failure for immediate learning
            if obs.back().is_some_and(|o| !o.success) {
                return Some(category.clone());
            }
        }
        None
    }

    /// Take observations for a category (consumes them)
    pub fn take_observations(&mut self, category: &str) -> Vec<ToolObservation> {
        self.observations
            .remove(category)
            .map(|v| v.into_iter().collect())
            .unwrap_or_default()
    }

    /// Add an episode to the buffer
    pub fn add_episode(&mut self, episode: Episode) {
        let category = episode.task_category.clone();
        self.episodes.entry(category).or_default().push(episode);
    }

    /// Check if any category has enough episodes for lesson synthesis
    pub fn ready_for_lesson_synthesis(&self) -> Option<String> {
        for (category, eps) in &self.episodes {
            if eps.len() >= self.episode_threshold {
                return Some(category.clone());
            }
        }
        None
    }

    /// Take episodes for a category (consumes them)
    pub fn take_episodes(&mut self, category: &str) -> Vec<Episode> {
        self.episodes.remove(category).unwrap_or_default()
    }

    /// Infer task category from tool name
    fn infer_category(tool_name: &str) -> String {
        let lower = tool_name.to_lowercase();
        if lower.contains("sector") || lower.contains("navigate") || lower.contains("move") {
            "navigation".to_string()
        } else if lower.contains("hazard") || lower.contains("danger") || lower.contains("crash") {
            "hazard_response".to_string()
        } else if lower.contains("build") || lower.contains("craft") || lower.contains("create") {
            "construction".to_string()
        } else {
            "general".to_string()
        }
    }
}

/// Central bus connecting event capture to learning and gossip
pub struct LearningBus {
    agent_id: String,
    swarm_id: String,
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
    /// Router for LLM calls during synthesis
    router: Option<Arc<Router>>,
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
            event_tx,
            event_rx: Arc::new(RwLock::new(Some(event_rx))),
            gossip,
            learning,
            keypair,
            gossip_out_tx,
            peer_addresses: Arc::new(RwLock::new(HashMap::new())),
            policy_cache: Arc::new(RwLock::new(PolicyCache::new())),
            episode_buffer: Arc::new(RwLock::new(EpisodeBuffer::new())),
            router: None,
        }
    }

    /// Set the router for LLM-based synthesis
    pub fn set_router(&mut self, router: Arc<Router>) {
        self.router = Some(router);
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
    pub async fn get_all_peer_addresses(&self) -> std::collections::HashMap<String, String> {
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

    /// Announce a lesson to the gossip network (signs before sending)
    pub async fn announce_lesson(
        &self,
        mut announcement: LessonAnnouncement,
    ) -> Result<(), String> {
        // Sign the announcement with our keypair
        sign_lesson_announcement(&mut announcement, &self.keypair)
            .map_err(|e| format!("Failed to sign lesson: {}", e))?;

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
    /// Note: The GossipProtocol emits these when a lesson reaches quorum
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
        let router = self.router.as_ref().ok_or("Router not configured")?;

        // Build prompt for LLM
        let obs_json = observations
            .iter()
            .map(|o| {
                serde_json::json!({
                    "tool": o.tool_name,
                    "args": o.args,
                    "result": o.result,
                    "success": o.success,
                    "latency_ms": o.latency_ms
                })
            })
            .collect::<Vec<_>>();

        let has_failure = observations.iter().any(|o| !o.success);
        let total_latency: u64 = observations.iter().map(|o| o.latency_ms).sum();

        let prompt = format!(
            r#"Analyze these tool call observations and summarize as a structured episode.

Category: {}
Observations:
{}

The sequence {} a failure.

Respond with a JSON object:
{{
  "summary": "brief description of what happened",
  "key_insight": "most important learning from this sequence",
  "quality_score": 0.0-1.0 based on success and efficiency
}}"#,
            category,
            serde_json::to_string_pretty(&obs_json).unwrap_or_default(),
            if has_failure {
                "contains"
            } else {
                "does not contain"
            }
        );

        let messages = vec![Message::user(prompt)];
        let stream = router
            .route(
                "Analyze observations and create episode summary",
                messages,
                None,
            )
            .await
            .map_err(|e| format!("Router error: {}", e))?;

        let response = stream
            .complete()
            .await
            .map_err(|e| format!("Stream error: {}", e))?;

        // Parse LLM response - extract quality score if present
        let quality_score = Self::extract_quality_score(&response.content);

        // Create episode
        let observation = Observation::new(
            serde_json::json!({"observations": obs_json}),
            format!("{} tool calls in {}", observations.len(), category),
            serde_json::json!({"summary": response.content}),
            observations.iter().map(|o| o.tool_name.clone()).collect(),
        );

        let outcome = EpisodeOutcome::new(
            !has_failure,
            QualityMetrics::new(quality_score, quality_score, 1.0),
            total_latency,
            response.cost_usd,
        );

        Ok(Episode::new(
            self.agent_id.clone(),
            self.swarm_id.clone(),
            Uuid::new_v4(),
            category.to_string(),
            observation,
            outcome,
        ))
    }

    /// Synthesize a lesson from episodes using LLM
    pub async fn synthesize_lesson(
        &self,
        episodes: &[Episode],
        category: &str,
    ) -> Result<Option<Lesson>, String> {
        let router = self.router.as_ref().ok_or("Router not configured")?;

        // Build prompt for LLM
        let eps_json = episodes
            .iter()
            .map(|e| {
                serde_json::json!({
                    "category": e.task_category,
                    "action": e.observation.action_taken,
                    "success": e.outcome.success,
                    "quality": e.outcome.quality_metrics.correctness,
                    "tools": e.observation.tools_used
                })
            })
            .collect::<Vec<_>>();

        let failures: Vec<_> = episodes.iter().filter(|e| !e.outcome.success).collect();
        let successes: Vec<_> = episodes.iter().filter(|e| e.outcome.success).collect();

        let prompt = format!(
            r#"Analyze these episodes and extract a reusable lesson pattern.

Category: {}
Episodes: {} total ({} successes, {} failures)
{}

Look for patterns:
- What conditions led to failures?
- What actions prevented failures?
- What invariants should be maintained?

If there's a clear pattern, respond with JSON:
{{
  "condition": "when this applies (e.g., sector_4)",
  "action": "recommended action (e.g., slow, avoid)",
  "expected_outcome": "what should happen",
  "confidence": 0.0-1.0 based on evidence strength
}}

If no clear pattern, respond with: NO_LESSON"#,
            category,
            episodes.len(),
            successes.len(),
            failures.len(),
            serde_json::to_string_pretty(&eps_json).unwrap_or_default()
        );

        let messages = vec![Message::user(prompt)];
        let stream = router
            .route("Extract lesson pattern from episodes", messages, None)
            .await
            .map_err(|e| format!("Router error: {}", e))?;

        let response = stream
            .complete()
            .await
            .map_err(|e| format!("Stream error: {}", e))?;

        // Check for no lesson
        if response.content.contains("NO_LESSON") {
            tracing::debug!("LLM found no pattern in {} episodes", episodes.len());
            return Ok(None);
        }

        // Parse the lesson pattern from response
        let pattern = Self::parse_lesson_pattern(&response.content)?;

        // Validate confidence
        if pattern.2 < 0.5 {
            tracing::debug!("Lesson confidence too low: {}", pattern.2);
            return Ok(None);
        }

        let lesson = Lesson::new(
            self.agent_id.clone(),
            self.swarm_id.clone(),
            category.to_string(),
            LessonPattern::new(pattern.0, pattern.1, pattern.3),
            pattern.2,
            episodes.len() as u32,
        );

        Ok(Some(lesson))
    }

    /// Extract quality score from LLM response
    fn extract_quality_score(content: &str) -> f64 {
        // Try to find quality_score in JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(score) = json.get("quality_score").and_then(|v| v.as_f64()) {
                return score.clamp(0.0, 1.0);
            }
        }
        // Default to 0.7 if we can't parse
        0.7
    }

    /// Parse lesson pattern from LLM response
    fn parse_lesson_pattern(content: &str) -> Result<(String, String, f64, String), String> {
        // Try to extract JSON from content (may have markdown wrapping)
        let json_str = if let Some(start) = content.find('{') {
            if let Some(end) = content.rfind('}') {
                &content[start..=end]
            } else {
                content
            }
        } else {
            content
        };

        let json: serde_json::Value =
            serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {}", e))?;

        let condition = json
            .get("condition")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let action = json
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("slow")
            .to_string();
        let confidence = json
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);
        let expected_outcome = json
            .get("expected_outcome")
            .and_then(|v| v.as_str())
            .unwrap_or("improved_outcome")
            .to_string();

        Ok((condition, action, confidence, expected_outcome))
    }
}

/// Start the lesson application loop that processes approved lessons
///
/// This function should be spawned as a background task. It listens for
/// lesson approvals from the gossip protocol and adds them to the policy cache.
pub async fn start_lesson_application_loop(
    learning_bus: Arc<LearningBus>,
    mut lesson_rx: broadcast::Receiver<LessonAnnouncement>,
) {
    tracing::info!("Starting lesson application loop");

    loop {
        match lesson_rx.recv().await {
            Ok(announcement) => {
                tracing::info!(
                    "Received approved lesson {}: {} from {}",
                    announcement.lesson_id,
                    announcement.category,
                    announcement.originator
                );

                // Convert announcement to Lesson and add to cache
                let lesson = Lesson::new(
                    announcement.originator.clone(),
                    learning_bus.swarm_id().to_string(),
                    announcement.category.clone(),
                    LessonPattern::new(
                        announcement.category.clone(),
                        "slow".to_string(),
                        "avoid_crash".to_string(),
                    ),
                    announcement.confidence,
                    1,
                );

                learning_bus.add_lesson_to_cache(lesson).await;

                tracing::info!(
                    "Added lesson {} to policy cache (total: {})",
                    announcement.lesson_id,
                    learning_bus.cached_lesson_count().await
                );
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Lesson application loop lagged by {} messages", n);
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!("Lesson application loop channel closed, exiting");
                break;
            }
        }
    }
}

/// Start the event processing loop that converts observations to episodes and lessons
///
/// This function should be spawned as a background task. It:
/// 1. Receives LearningEvents from tool calls
/// 2. Buffers observations
/// 3. Synthesizes episodes when threshold reached
/// 4. Synthesizes lessons from episodes
/// 5. Announces lessons via gossip
pub async fn start_event_processing_loop(
    learning_bus: Arc<LearningBus>,
    mut event_rx: mpsc::Receiver<LearningEvent>,
) {
    tracing::info!("Starting event processing loop");

    loop {
        match event_rx.recv().await {
            Some(event) => {
                match event {
                    LearningEvent::ToolCall {
                        tool_name,
                        args,
                        result,
                        success,
                        latency_ms,
                    } => {
                        tracing::debug!(
                            "Event processing: tool={} success={} latency={}ms",
                            tool_name,
                            success,
                            latency_ms
                        );

                        // Create observation
                        let obs = ToolObservation {
                            tool_name,
                            args,
                            result,
                            success,
                            latency_ms,
                            timestamp: Utc::now(),
                        };

                        // Add to buffer
                        {
                            let mut buffer = learning_bus.episode_buffer.write().await;
                            buffer.add_observation(obs);
                        }

                        // Check if ready for episode synthesis
                        let category_ready = {
                            let buffer = learning_bus.episode_buffer.read().await;
                            buffer.ready_for_episode_synthesis()
                        };

                        if let Some(category) = category_ready {
                            let observations = {
                                let mut buffer = learning_bus.episode_buffer.write().await;
                                buffer.take_observations(&category)
                            };

                            tracing::info!(
                                "Synthesizing episode from {} observations in {}",
                                observations.len(),
                                category
                            );

                            match learning_bus
                                .synthesize_episode(&observations, &category)
                                .await
                            {
                                Ok(episode) => {
                                    tracing::info!(
                                        "Episode synthesized: {} (success={})",
                                        episode.id,
                                        episode.outcome.success
                                    );

                                    // Add to buffer
                                    {
                                        let mut buffer = learning_bus.episode_buffer.write().await;
                                        buffer.add_episode(episode);
                                    }

                                    // Check if ready for lesson synthesis
                                    let lesson_category_ready = {
                                        let buffer = learning_bus.episode_buffer.read().await;
                                        buffer.ready_for_lesson_synthesis()
                                    };

                                    if let Some(lesson_category) = lesson_category_ready {
                                        let episodes = {
                                            let mut buffer =
                                                learning_bus.episode_buffer.write().await;
                                            buffer.take_episodes(&lesson_category)
                                        };

                                        tracing::info!(
                                            "Synthesizing lesson from {} episodes in {}",
                                            episodes.len(),
                                            lesson_category
                                        );

                                        match learning_bus
                                            .synthesize_lesson(&episodes, &lesson_category)
                                            .await
                                        {
                                            Ok(Some(lesson)) => {
                                                tracing::info!(
                                                    "Lesson synthesized: {} confidence={}",
                                                    lesson.pattern.condition,
                                                    lesson.confidence
                                                );

                                                // Create announcement and gossip
                                                let announcement = LessonAnnouncement::new(
                                                    lesson.id,
                                                    [0u8; 32], // Hash computed during signing
                                                    learning_bus.agent_id.clone(),
                                                    learning_bus.swarm_id.clone(),
                                                    lesson.category.clone(),
                                                    lesson.confidence,
                                                );

                                                if let Err(e) =
                                                    learning_bus.announce_lesson(announcement).await
                                                {
                                                    tracing::error!(
                                                        "Failed to announce lesson: {}",
                                                        e
                                                    );
                                                }
                                            }
                                            Ok(None) => {
                                                tracing::debug!(
                                                    "No lesson pattern found in {} episodes",
                                                    episodes.len()
                                                );
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to synthesize lesson: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to synthesize episode: {}", e);
                                }
                            }
                        }
                    }
                    LearningEvent::TaskComplete {
                        task_id,
                        category,
                        success,
                        ..
                    } => {
                        tracing::debug!(
                            "Task complete: {} category={} success={}",
                            task_id,
                            category,
                            success
                        );
                        // Task completion can trigger immediate episode synthesis
                        // even before threshold is reached
                    }
                    LearningEvent::GossipReceived(_) => {
                        // Gossip messages are handled separately via handle_gossip
                    }
                }
            }
            None => {
                tracing::info!("Event processing loop channel closed, exiting");
                break;
            }
        }
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
