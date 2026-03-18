//! Feedback loop types: quality gate, policy cache, gossip, consolidation, resilience (§7).

use serde::{Deserialize, Serialize};

/// Multi-timescale feedback system (§7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackLoops {
    pub immediate: ImmediateFeedback,
    pub short_term: ShortTermFeedback,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gossip: Option<Gossip>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consolidation: Option<Consolidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resilience: Option<Resilience>,
}

/// Per-event feedback (§7.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImmediateFeedback {
    pub quality_gate: QualityGate,
}

/// Quality gate configuration (§7.1.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityGate {
    pub threshold_default: f64,
    pub metric: QualityMetric,
    pub on_failure: QualityFailureAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_overrides: Option<Vec<ThresholdOverride>>,
}

/// Quality measurement method (§7.1.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualityMetric {
    CosineSimilarity,
    LlmJudge,
    RegexMatch,
    Composite,
}

/// Action when quality is below threshold (§7.1.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualityFailureAction {
    UpdatePriorAndLog,
    UpdatePriorAndRetry,
    LogOnly,
}

/// Per-(task_class, entity) threshold override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdOverride {
    pub task_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    pub threshold: f64,
}

/// PolicyCache configuration (§7.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortTermFeedback {
    pub policy_cache: PolicyCacheConfig,
}

/// PolicyCache behavior (§7.2.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCacheConfig {
    pub default_ttl_sec: u64,
    pub decay_strategy: DecayStrategy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decay_half_life_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_source_exempt_from_decay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incident_source_quarantine_sec: Option<u64>,
}

/// Decay strategy for PolicyCache entries (§7.2.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecayStrategy {
    Exponential,
    Linear,
    Step,
    None,
}

/// Peer-to-peer policy propagation (§7.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gossip {
    pub propagation_interval_sec: u64,
    pub peer_discount_factor: f64,
    pub require_did_signature: bool,
    pub quorum: Quorum,
}

/// Sybil resistance for gossip (§7.3.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quorum {
    pub min_trusted_peers: u32,
    pub trusted_registry: String,
    pub untrusted_did_weight: f64,
    pub new_did_probation_period_sec: u64,
}

/// Offline learning phase (§7.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consolidation {
    pub schedule: String,
    pub runner: ConsolidationRunner,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_preference: Option<String>,
    pub operations: Vec<ConsolidationOp>,
    pub axiom_store: AxiomStoreConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_signature: Option<bool>,
}

/// Consolidation runner type (§7.4).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationRunner {
    Conductor,
    DedicatedNode,
}

/// Consolidation operations executed in order (§7.4).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationOp {
    LessonDeduplication,
    PatternExtraction,
    ContradictionDetection,
    AxiomFalsifiabilityCheck,
    CuriosityTaskGeneration,
}

/// Axiom store configuration (§7.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxiomStoreConfig {
    pub max_axioms: u32,
    pub falsification_window_days: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_consolidation_signature: Option<bool>,
}

/// Retry and circuit breaker policy (§7.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resilience {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit_breaker: Option<CircuitBreaker>,
}

/// Retry policy (§7.5.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries_default: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backoff_strategy: Option<BackoffStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_budget_per_minute: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_overrides: Option<Vec<serde_json::Value>>,
}

/// Backoff strategy for retries (§7.5.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackoffStrategy {
    Fixed,
    Linear,
    Exponential,
}

/// Circuit breaker configuration (§7.5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreaker {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_threshold: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_timeout_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_threshold: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_overrides: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_to_gossip: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_gate_serialization() {
        let gate = QualityGate {
            threshold_default: 0.7,
            metric: QualityMetric::CosineSimilarity,
            on_failure: QualityFailureAction::UpdatePriorAndLog,
            threshold_overrides: None,
        };
        let json = serde_json::to_string(&gate).unwrap();
        assert!(json.contains("cosine_similarity"));
        assert!(json.contains("update_prior_and_log"));
    }

    #[test]
    fn decay_strategy_variants() {
        for (input, expected) in [
            (r#""exponential""#, DecayStrategy::Exponential),
            (r#""linear""#, DecayStrategy::Linear),
            (r#""step""#, DecayStrategy::Step),
            (r#""none""#, DecayStrategy::None),
        ] {
            let parsed: DecayStrategy = serde_json::from_str(input).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn resilience_with_retry_and_circuit_breaker() {
        let json = r#"{
            "retry": {
                "max_retries_default": 3,
                "backoff_strategy": "exponential",
                "initial_delay_ms": 500,
                "max_delay_ms": 30000,
                "retry_budget_per_minute": 20
            },
            "circuit_breaker": {
                "failure_threshold": 5,
                "recovery_timeout_sec": 60,
                "success_threshold": 2,
                "report_to_gossip": true
            }
        }"#;
        let resilience: Resilience = serde_json::from_str(json).unwrap();
        let retry = resilience.retry.unwrap();
        assert_eq!(retry.max_retries_default, Some(3));
        let cb = resilience.circuit_breaker.unwrap();
        assert_eq!(cb.failure_threshold, Some(5));
    }
}
