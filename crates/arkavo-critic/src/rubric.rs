//! Calibrated evaluation rubric for graded critic scoring
//!
//! Provides four-dimension scoring that feeds Thompson Sampling
//! with continuous signals rather than binary pass/fail.

use serde::{Deserialize, Serialize};

/// Weights for the four rubric dimensions
///
/// Must sum to 1.0 for correct composite scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RubricWeights {
    pub goal_fidelity: f64,
    pub efficiency: f64,
    pub state_coherence: f64,
    pub adaptability: f64,
}

impl Default for RubricWeights {
    fn default() -> Self {
        Self {
            goal_fidelity: 0.35,
            efficiency: 0.25,
            state_coherence: 0.25,
            adaptability: 0.15,
        }
    }
}

impl RubricWeights {
    /// Validate that weights sum to approximately 1.0
    pub fn is_valid(&self) -> bool {
        let sum = self.goal_fidelity + self.efficiency + self.state_coherence + self.adaptability;
        (sum - 1.0).abs() < 0.01
    }
}

/// Four-dimension evaluation rubric
///
/// Each dimension scores 0.0-1.0. The composite weighted average
/// feeds Thompson Sampling as a continuous learning signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationRubric {
    /// Did the output achieve the task's actual intent?
    pub goal_fidelity: f64,
    /// Was inference budget used efficiently?
    pub efficiency: f64,
    /// Is the resulting state consistent and contradiction-free?
    pub state_coherence: f64,
    /// Did the operator handle unexpected conditions gracefully?
    pub adaptability: f64,
}

impl EvaluationRubric {
    pub fn new(
        goal_fidelity: f64,
        efficiency: f64,
        state_coherence: f64,
        adaptability: f64,
    ) -> Self {
        Self {
            goal_fidelity: goal_fidelity.clamp(0.0, 1.0),
            efficiency: efficiency.clamp(0.0, 1.0),
            state_coherence: state_coherence.clamp(0.0, 1.0),
            adaptability: adaptability.clamp(0.0, 1.0),
        }
    }

    /// Weighted composite score using the given weights
    pub fn composite(&self, weights: &RubricWeights) -> f64 {
        self.adaptability.mul_add(
            weights.adaptability,
            self.state_coherence.mul_add(
                weights.state_coherence,
                self.goal_fidelity
                    .mul_add(weights.goal_fidelity, self.efficiency * weights.efficiency),
            ),
        )
    }

    /// Map composite score to Thompson Sampling beta distribution update.
    ///
    /// Returns (alpha_add, beta_add):
    /// - Composite >= 0.7: strong alpha boost (good performance)
    /// - Composite 0.5-0.7: neutral (slight alpha)
    /// - Composite < 0.5: beta boost (poor performance)
    pub fn to_beta_update(&self, weights: &RubricWeights) -> (f64, f64) {
        let c = self.composite(weights);
        if c >= 0.7 {
            let alpha_boost = (c - 0.7) / 0.3; // 0.0-1.0
            (1.0 + alpha_boost, 0.0)
        } else if c < 0.5 {
            let beta_boost = (0.5 - c) / 0.5; // 0.0-1.0
            (0.0, 1.0 + beta_boost)
        } else {
            (0.2, 0.0) // neutral: slight alpha bump
        }
    }

    /// Construct from generic domain-specific (score, weight) pairs.
    ///
    /// Pairs are mapped positionally to [goal_fidelity, efficiency,
    /// state_coherence, adaptability]. Extra pairs are ignored;
    /// missing dimensions default to 0.5.
    pub fn from_domain_scores(pairs: &[(f64, f64)]) -> Self {
        let extract = |idx: usize| -> f64 {
            pairs
                .get(idx)
                .map_or(0.5, |(score, _)| score.clamp(0.0, 1.0))
        };
        Self {
            goal_fidelity: extract(0),
            efficiency: extract(1),
            state_coherence: extract(2),
            adaptability: extract(3),
        }
    }

    /// Extract domain weights from the same pairs used in from_domain_scores
    pub fn weights_from_domain(pairs: &[(f64, f64)]) -> RubricWeights {
        let total: f64 = pairs.iter().take(4).map(|(_, w)| w.abs()).sum();
        if total < f64::EPSILON {
            return RubricWeights::default();
        }
        let w = |idx: usize| -> f64 {
            pairs
                .get(idx)
                .map_or(0.0, |(_, weight)| weight.abs() / total)
        };
        RubricWeights {
            goal_fidelity: w(0),
            efficiency: w(1),
            state_coherence: w(2),
            adaptability: w(3),
        }
    }
}

/// Trait for domain-specific evaluation (game sessions, etc.)
///
/// Implementations live outside core crates (e.g., in examples/).
/// The trait converts raw JSON observations from MCP tools into
/// a generic four-dimension rubric.
pub trait DomainEvaluator: Send + Sync {
    /// Evaluate observations and produce a rubric
    fn evaluate(&self, observations: &[serde_json::Value]) -> EvaluationRubric;

    /// Domain name (e.g., "rimworld", "factorio")
    fn domain_name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_weights_valid() {
        let w = RubricWeights::default();
        assert!(w.is_valid());
        let sum = w.goal_fidelity + w.efficiency + w.state_coherence + w.adaptability;
        assert!((sum - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_invalid_weights() {
        let w = RubricWeights {
            goal_fidelity: 0.5,
            efficiency: 0.5,
            state_coherence: 0.5,
            adaptability: 0.5,
        };
        assert!(!w.is_valid());
    }

    #[test]
    fn test_rubric_clamping() {
        let r = EvaluationRubric::new(1.5, -0.1, 0.5, 0.8);
        assert_eq!(r.goal_fidelity, 1.0);
        assert_eq!(r.efficiency, 0.0);
        assert_eq!(r.state_coherence, 0.5);
        assert_eq!(r.adaptability, 0.8);
    }

    #[test]
    fn test_composite_with_defaults() {
        let w = RubricWeights::default();
        let r = EvaluationRubric::new(1.0, 1.0, 1.0, 1.0);
        let c = r.composite(&w);
        assert!((c - 1.0).abs() < f64::EPSILON);

        let r = EvaluationRubric::new(0.0, 0.0, 0.0, 0.0);
        assert!((r.composite(&w)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_composite_weighted() {
        let w = RubricWeights::default();
        // Only goal_fidelity = 1.0, rest = 0.0
        let r = EvaluationRubric::new(1.0, 0.0, 0.0, 0.0);
        assert!((r.composite(&w) - 0.35).abs() < f64::EPSILON);
    }

    #[test]
    fn test_beta_update_high_score() {
        let w = RubricWeights::default();
        let r = EvaluationRubric::new(1.0, 1.0, 1.0, 1.0);
        let (alpha, beta) = r.to_beta_update(&w);
        assert!(alpha > 1.0);
        assert_eq!(beta, 0.0);
    }

    #[test]
    fn test_beta_update_low_score() {
        let w = RubricWeights::default();
        let r = EvaluationRubric::new(0.0, 0.0, 0.0, 0.0);
        let (alpha, beta) = r.to_beta_update(&w);
        assert_eq!(alpha, 0.0);
        assert!(beta > 1.0);
    }

    #[test]
    fn test_beta_update_neutral() {
        let w = RubricWeights::default();
        let r = EvaluationRubric::new(0.6, 0.6, 0.6, 0.6);
        let (alpha, beta) = r.to_beta_update(&w);
        assert_eq!(alpha, 0.2);
        assert_eq!(beta, 0.0);
    }

    #[test]
    fn test_from_domain_scores_full() {
        let pairs = [(0.8, 0.3), (0.6, 0.25), (0.9, 0.25), (0.7, 0.2)];
        let r = EvaluationRubric::from_domain_scores(&pairs);
        assert_eq!(r.goal_fidelity, 0.8);
        assert_eq!(r.efficiency, 0.6);
        assert_eq!(r.state_coherence, 0.9);
        assert_eq!(r.adaptability, 0.7);
    }

    #[test]
    fn test_from_domain_scores_partial() {
        let pairs = [(0.8, 0.5), (0.6, 0.5)];
        let r = EvaluationRubric::from_domain_scores(&pairs);
        assert_eq!(r.goal_fidelity, 0.8);
        assert_eq!(r.efficiency, 0.6);
        assert_eq!(r.state_coherence, 0.5); // default
        assert_eq!(r.adaptability, 0.5); // default
    }

    #[test]
    fn test_weights_from_domain() {
        let pairs = [(0.8, 0.35), (0.6, 0.25), (0.9, 0.25), (0.7, 0.15)];
        let w = EvaluationRubric::weights_from_domain(&pairs);
        assert!(w.is_valid());
    }

    #[test]
    fn test_weights_from_domain_empty() {
        let w = EvaluationRubric::weights_from_domain(&[]);
        assert!(w.is_valid()); // falls back to default
    }

    #[test]
    fn test_serialization_roundtrip() {
        let r = EvaluationRubric::new(0.8, 0.6, 0.9, 0.7);
        let json = serde_json::to_string(&r).unwrap();
        let r2: EvaluationRubric = serde_json::from_str(&json).unwrap();
        assert_eq!(r.goal_fidelity, r2.goal_fidelity);
        assert_eq!(r.efficiency, r2.efficiency);
    }
}
