//! Adaptation engine types: method selection, cold start, prior management, signal separation (§6).

use serde::{Deserialize, Serialize};

use crate::model::BetaPrior;

/// Core adaptation engine configuration (§6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Adaptation {
    pub method: AdaptationMethod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<AdaptationParameters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold_start: Option<ColdStart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_management: Option<PriorManagement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_separation: Option<SignalSeparation>,
}

/// Available adaptation methods (§6.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdaptationMethod {
    ThompsonSampling,
    EpsilonGreedy,
    Ucb1,
    Static,
}

/// Method-specific parameters (§6.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptationParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exploration_floor: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epsilon: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epsilon_decay: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epsilon_min: Option<f64>,
}

/// Cold start policy (§6.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColdStart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<ColdStartStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_prior: Option<BetaPrior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_period: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_behavior: Option<WarmupBehavior>,
}

/// Cold start strategy options (§6.2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColdStartStrategy {
    Optimistic,
    Pessimistic,
    Neutral,
    ConstitutionalOnly,
}

/// Warmup behavior during cold start (§6.2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WarmupBehavior {
    ConstitutionalOnly,
    ExploreWithGuard,
    Normal,
}

/// Prior version binding and reset rules (§6.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorManagement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_binding: Option<VersionBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_on_version_change: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_state: Option<BetaPrior>,
    /// Track per-observation provenance (live vs distilled). Default false:
    /// distilled mass merges into the live prior, exactly as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_tracking: Option<bool>,
    /// How distilled (teacher-provided) mass decays as live evidence
    /// accumulates. Only meaningful when `provenance_tracking` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distilled_decay: Option<DistilledDecay>,
}

/// Decay of distilled prior mass under accumulating live evidence (§6.5).
///
/// The `BetaPrior` wire format is unchanged: provenance exists only in
/// engine state and snapshots; the effective prior sampled by the engine is
/// `live + decayed distilled` mass.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DistilledDecay {
    pub strategy: DistilledDecayStrategy,
    /// Per-live-observation reduction of the distilled weight. The weight is
    /// `max(floor, 1 - displacement_factor * live_observations)`. Must be in
    /// (0, 1]. Default 0.05.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displacement_factor: Option<f64>,
    /// Minimum distilled weight that always survives. Must be in [0, 1].
    /// Default 0.0 (live evidence can fully displace distilled mass).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<f64>,
}

/// Distilled-decay strategy (§6.5).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DistilledDecayStrategy {
    /// Live observations linearly displace distilled mass.
    LiveDisplacement,
}

/// How priors are bound to entity versions (§6.3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VersionBinding {
    EntityHash,
    SemanticVersion,
    NameOnly,
}

/// Availability vs quality signal separation (§6.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalSeparation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_failures_update_priors: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_cooldown_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_failure_types: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptation_method_serialization() {
        let method = AdaptationMethod::ThompsonSampling;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, r#""thompson_sampling""#);
    }

    #[test]
    fn minimal_adaptation() {
        let json = r#"{"method": "static"}"#;
        let adaptation: Adaptation = serde_json::from_str(json).unwrap();
        assert_eq!(adaptation.method, AdaptationMethod::Static);
        assert!(adaptation.parameters.is_none());
    }

    #[test]
    fn full_adaptation() {
        let json = r#"{
            "method": "thompson_sampling",
            "parameters": {"exploration_floor": 0.05},
            "cold_start": {
                "strategy": "optimistic",
                "initial_prior": {"alpha": 2.0, "beta": 1.0},
                "warmup_period": 5,
                "warmup_behavior": "constitutional_only"
            },
            "prior_management": {
                "version_binding": "entity_hash",
                "reset_on_version_change": true,
                "reset_state": {"alpha": 2.0, "beta": 1.0}
            },
            "signal_separation": {
                "availability_failures_update_priors": false,
                "availability_cooldown_sec": 300,
                "availability_failure_types": ["timeout", "connection_refused"]
            }
        }"#;
        let adaptation: Adaptation = serde_json::from_str(json).unwrap();
        assert_eq!(adaptation.method, AdaptationMethod::ThompsonSampling);
        let params = adaptation.parameters.unwrap();
        assert_eq!(params.exploration_floor, Some(0.05));
        let cold = adaptation.cold_start.unwrap();
        assert_eq!(cold.strategy, Some(ColdStartStrategy::Optimistic));
    }
}
