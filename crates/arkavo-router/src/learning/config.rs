//! Configuration for the learning module

use serde::{Deserialize, Serialize};

/// Configuration for Thompson Sampling and credit assignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningConfig {
    /// Weight for semantic similarity in scoring (α)
    pub semantic_weight: f64,
    /// Weight for skill/capability overlap in scoring (β)
    pub skill_weight: f64,
    /// Weight for utility prior in scoring (γ)
    pub utility_weight: f64,
    /// Discount factor for retrospective credit assignment (γ)
    pub discount_factor: f64,
    /// Minimum observations before graduating from probation
    pub probation_threshold: u64,
    /// Maximum recent bursts to track per agent
    pub max_recent_bursts: usize,
    /// Decay factor for concept drift (applied periodically)
    pub decay_factor: f64,
    /// How often to apply decay (in observations)
    pub decay_interval: u64,
    /// Exploration bonus for high-uncertainty agents
    pub exploration_bonus: f64,
    /// Budget allocation for probationary agents (0.0 to 1.0)
    pub probationary_budget_ratio: f64,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            semantic_weight: 0.3,
            skill_weight: 0.3,
            utility_weight: 0.4,
            discount_factor: 0.95,
            probation_threshold: 50,
            max_recent_bursts: 100,
            decay_factor: 0.99,
            decay_interval: 1000,
            exploration_bonus: 0.1,
            probationary_budget_ratio: 0.01,
        }
    }
}

impl LearningConfig {
    /// Create a new config with custom weights
    pub fn with_weights(semantic: f64, skill: f64, utility: f64) -> Self {
        let total = semantic + skill + utility;
        Self {
            semantic_weight: semantic / total,
            skill_weight: skill / total,
            utility_weight: utility / total,
            ..Default::default()
        }
    }

    /// Set the discount factor for retrospective credit
    pub fn with_discount_factor(mut self, factor: f64) -> Self {
        self.discount_factor = factor.clamp(0.0, 1.0);
        self
    }

    /// Set the probation threshold
    pub fn with_probation_threshold(mut self, threshold: u64) -> Self {
        self.probation_threshold = threshold;
        self
    }

    /// Calculate combined score from components
    pub fn score(&self, semantic_sim: f64, skill_overlap: f64, utility_sample: f64) -> f64 {
        self.utility_weight.mul_add(
            utility_sample,
            self.semantic_weight
                .mul_add(semantic_sim, self.skill_weight * skill_overlap),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_weights_sum_to_one() {
        let config = LearningConfig::default();
        let sum = config.semantic_weight + config.skill_weight + config.utility_weight;
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_custom_weights_normalize() {
        let config = LearningConfig::with_weights(1.0, 2.0, 2.0);
        let sum = config.semantic_weight + config.skill_weight + config.utility_weight;
        assert!((sum - 1.0).abs() < 0.001);
        assert!((config.semantic_weight - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_score_calculation() {
        let config = LearningConfig::with_weights(0.3, 0.3, 0.4);
        let score = config.score(0.8, 0.7, 0.9);
        // 0.3*0.8 + 0.3*0.7 + 0.4*0.9 = 0.24 + 0.21 + 0.36 = 0.81
        assert!((score - 0.81).abs() < 0.01);
    }
}
