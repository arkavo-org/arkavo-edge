//! Episodic memory and lesson persistence types for cross-swarm learning

use super::QualityMetrics;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Where a lesson originated — determines decay and override behavior
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LessonSource {
    /// Learned from accumulated episodes via synthesis pipeline
    #[default]
    Machine,
    /// Provided by a human operator at runtime (immune to decay)
    Human,
    /// Loaded from AGENTS.md at startup
    Configuration,
}

impl std::fmt::Display for LessonSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Machine => write!(f, "machine"),
            Self::Human => write!(f, "human"),
            Self::Configuration => write!(f, "configuration"),
        }
    }
}

impl std::str::FromStr for LessonSource {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "machine" => Ok(Self::Machine),
            "human" => Ok(Self::Human),
            "configuration" => Ok(Self::Configuration),
            _ => Err(format!("unknown lesson source: {s}")),
        }
    }
}

/// Scope of a lesson's applicability
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LessonScope {
    /// Applies to all future tasks
    #[default]
    Global,
    /// Applies only when the condition matches
    Situational,
    /// Execute once, then expire
    OneShot,
}

impl std::fmt::Display for LessonScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => write!(f, "global"),
            Self::Situational => write!(f, "situational"),
            Self::OneShot => write!(f, "one_shot"),
        }
    }
}

impl std::str::FromStr for LessonScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "global" => Ok(Self::Global),
            "situational" => Ok(Self::Situational),
            "one_shot" => Ok(Self::OneShot),
            _ => Err(format!("unknown lesson scope: {s}")),
        }
    }
}

/// Memory tier for lifecycle management
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryTier {
    /// Recent, unvalidated data (expires after transient_ttl)
    #[default]
    Transient,
    /// Validated by multiple successes, pending promotion
    Candidate,
    /// Proven pattern, long-lived
    Canonical,
}

impl std::fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient => write!(f, "transient"),
            Self::Candidate => write!(f, "candidate"),
            Self::Canonical => write!(f, "canonical"),
        }
    }
}

impl std::str::FromStr for MemoryTier {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "transient" => Ok(Self::Transient),
            "candidate" => Ok(Self::Candidate),
            "canonical" => Ok(Self::Canonical),
            _ => Err(format!("unknown memory tier: {s}")),
        }
    }
}

/// Episodic memory record - individual learning experience
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    /// Unique episode identifier
    pub id: Uuid,
    /// Agent that experienced this episode
    pub agent_id: String,
    /// Swarm this agent belongs to
    pub swarm_id: String,
    /// Task this episode was part of
    pub task_id: Uuid,
    /// Category of the task
    pub task_category: String,
    /// What was observed
    pub observation: Observation,
    /// How it turned out
    pub outcome: EpisodeOutcome,
    /// When this episode occurred
    pub timestamp: DateTime<Utc>,
    /// Hash for deduplication
    pub context_hash: [u8; 32],
    /// Lifecycle tier
    #[serde(default)]
    pub tier: MemoryTier,
}

impl Episode {
    /// Create a new episode
    pub fn new(
        agent_id: String,
        swarm_id: String,
        task_id: Uuid,
        task_category: String,
        observation: Observation,
        outcome: EpisodeOutcome,
    ) -> Self {
        let mut episode = Self {
            id: Uuid::new_v4(),
            agent_id,
            swarm_id,
            task_id,
            task_category,
            observation,
            outcome,
            timestamp: Utc::now(),
            context_hash: [0u8; 32],
            tier: MemoryTier::Transient,
        };
        episode.context_hash = episode.compute_hash();
        episode
    }

    /// Compute a hash of the episode context for deduplication
    fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.agent_id.as_bytes());
        hasher.update(self.task_category.as_bytes());
        hasher.update(self.observation.action_taken.as_bytes());
        if let Ok(state_bytes) = serde_json::to_vec(&self.observation.state_before) {
            hasher.update(&state_bytes);
        }
        hasher.finalize().into()
    }
}

/// Observation of an action taken in an environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// State before the action
    pub state_before: serde_json::Value,
    /// Action that was taken
    pub action_taken: String,
    /// State after the action
    pub state_after: serde_json::Value,
    /// Tools used during the action
    pub tools_used: Vec<String>,
}

impl Observation {
    /// Create a new observation
    pub fn new(
        state_before: serde_json::Value,
        action_taken: String,
        state_after: serde_json::Value,
        tools_used: Vec<String>,
    ) -> Self {
        Self {
            state_before,
            action_taken,
            state_after,
            tools_used,
        }
    }
}

/// Outcome of an episode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeOutcome {
    /// Whether the action succeeded
    pub success: bool,
    /// Quality metrics for the outcome
    pub quality_metrics: QualityMetrics,
    /// Latency in milliseconds
    pub latency_ms: u64,
    /// Cost incurred
    pub cost_usd: f64,
}

impl EpisodeOutcome {
    /// Create a new outcome
    pub fn new(
        success: bool,
        quality_metrics: QualityMetrics,
        latency_ms: u64,
        cost_usd: f64,
    ) -> Self {
        Self {
            success,
            quality_metrics,
            latency_ms,
            cost_usd,
        }
    }

    /// Create a successful outcome with minimal metrics
    pub fn success() -> Self {
        Self {
            success: true,
            quality_metrics: QualityMetrics::new(1.0, 1.0, 1.0),
            latency_ms: 0,
            cost_usd: 0.0,
        }
    }

    /// Create a failed outcome with minimal metrics
    pub fn failure() -> Self {
        Self {
            success: false,
            quality_metrics: QualityMetrics::new(0.0, 0.0, 0.0),
            latency_ms: 0,
            cost_usd: 0.0,
        }
    }
}

/// Lesson summary - distilled learning from multiple episodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    /// Unique lesson identifier
    pub id: Uuid,
    /// Agent that synthesized this lesson
    pub agent_id: String,
    /// Swarm this lesson applies to
    pub swarm_id: String,
    /// Category this lesson applies to
    pub category: String,
    /// The learned pattern
    pub pattern: LessonPattern,
    /// Confidence in this lesson (0.0-1.0)
    pub confidence: f64,
    /// Number of episodes supporting this lesson
    pub episode_count: u32,
    /// When this lesson was created
    pub created_at: DateTime<Utc>,
    /// When this lesson was last updated
    pub updated_at: DateTime<Utc>,
    /// Whether this lesson has been propagated to peers
    pub propagated: bool,
    /// Ed25519 signature for verification
    pub signature: Vec<u8>,
    /// Lifecycle tier
    #[serde(default)]
    pub tier: MemoryTier,
    /// Where this lesson originated
    #[serde(default)]
    pub source: LessonSource,
    /// How broadly this lesson applies
    #[serde(default)]
    pub scope: LessonScope,
}

impl Lesson {
    /// Create a new lesson
    pub fn new(
        agent_id: String,
        swarm_id: String,
        category: String,
        pattern: LessonPattern,
        confidence: f64,
        episode_count: u32,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            agent_id,
            swarm_id,
            category,
            pattern,
            confidence: confidence.clamp(0.0, 1.0),
            episode_count,
            created_at: now,
            updated_at: now,
            propagated: false,
            signature: Vec::new(),
            tier: MemoryTier::Transient,
            source: LessonSource::Machine,
            scope: LessonScope::Global,
        }
    }

    /// Set the lesson source
    pub fn with_source(mut self, source: LessonSource) -> Self {
        self.source = source;
        self
    }

    /// Set the lesson scope
    pub fn with_scope(mut self, scope: LessonScope) -> Self {
        self.scope = scope;
        self
    }

    /// Compute hash of lesson for gossip
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.id.as_bytes());
        hasher.update(self.agent_id.as_bytes());
        hasher.update(self.swarm_id.as_bytes());
        hasher.update(self.category.as_bytes());
        hasher.update(self.pattern.condition.as_bytes());
        hasher.update(self.pattern.action.as_bytes());
        hasher.finalize().into()
    }

    /// Get content bytes for signing
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.id.as_bytes());
        content.extend(self.agent_id.as_bytes());
        content.extend(self.swarm_id.as_bytes());
        content.extend(self.category.as_bytes());
        content.extend(self.pattern.condition.as_bytes());
        content.extend(self.pattern.action.as_bytes());
        content.extend(self.confidence.to_le_bytes());
        content.extend(self.episode_count.to_le_bytes());
        content.extend(self.created_at.timestamp().to_le_bytes());
        content
    }

    /// Mark as propagated
    pub fn mark_propagated(&mut self) {
        self.propagated = true;
        self.updated_at = Utc::now();
    }
}

/// A learned pattern from episodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonPattern {
    /// Condition under which this pattern applies
    pub condition: String,
    /// Action to take
    pub action: String,
    /// Expected outcome
    pub expected_outcome: String,
    /// Optional link to a safety invariant
    pub invariant_hash: Option<[u8; 32]>,
    /// Flexible metadata for lesson-type-specific data (tool formats, aliases, etc.)
    pub metadata: Option<serde_json::Value>,
}

impl LessonPattern {
    /// Create a new pattern
    pub fn new(condition: String, action: String, expected_outcome: String) -> Self {
        Self {
            condition,
            action,
            expected_outcome,
            invariant_hash: None,
            metadata: None,
        }
    }

    /// Set invariant hash
    pub fn with_invariant(mut self, hash: [u8; 32]) -> Self {
        self.invariant_hash = Some(hash);
        self
    }

    /// Set metadata for lesson-type-specific data
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_episode_creation() {
        let observation = Observation::new(
            serde_json::json!({"sector": 4, "hazard": false}),
            "enter_sector".to_string(),
            serde_json::json!({"sector": 4, "hazard": true, "crashed": true}),
            vec!["get_sector".to_string()],
        );

        let outcome = EpisodeOutcome::failure();

        let episode = Episode::new(
            "rover-alpha".to_string(),
            "fleet".to_string(),
            Uuid::new_v4(),
            "navigation".to_string(),
            observation,
            outcome,
        );

        assert_eq!(episode.agent_id, "rover-alpha");
        assert!(!episode.outcome.success);
        assert_ne!(episode.context_hash, [0u8; 32]);
    }

    #[test]
    fn test_lesson_creation() {
        let pattern = LessonPattern::new(
            "IF sector_4".to_string(),
            "drive_slow".to_string(),
            "avoid_crash".to_string(),
        );

        let lesson = Lesson::new(
            "rover-alpha".to_string(),
            "fleet".to_string(),
            "navigation".to_string(),
            pattern,
            0.9,
            5,
        );

        assert_eq!(lesson.confidence, 0.9);
        assert_eq!(lesson.episode_count, 5);
        assert!(!lesson.propagated);
        assert_ne!(lesson.compute_hash(), [0u8; 32]);
    }

    #[test]
    fn test_lesson_hash_consistency() {
        let pattern = LessonPattern::new("cond".to_string(), "act".to_string(), "out".to_string());

        let lesson1 = Lesson::new(
            "agent".to_string(),
            "swarm".to_string(),
            "cat".to_string(),
            pattern.clone(),
            0.8,
            3,
        );

        let mut lesson2 = lesson1.clone();
        lesson2.id = lesson1.id; // Same ID

        assert_eq!(lesson1.compute_hash(), lesson2.compute_hash());
    }
}
