//! Uncertainty estimation for routing decisions
//!
//! Combines model uncertainty (BetaPrior std_dev), retrieval agreement,
//! and tool contradiction signals to determine when to explore more
//! or ask for human review.

use serde::{Deserialize, Serialize};

/// Combined uncertainty estimate from multiple signals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncertaintyEstimate {
    /// BetaPrior.std_dev() for the selected model
    pub model_uncertainty: f64,
    /// Do similar past cases agree on approach? (1.0 = agree, 0.0 = disagree)
    pub retrieval_agreement: f64,
    /// Did tool results conflict? (0.0 = no conflict, 1.0 = high conflict)
    pub tool_contradiction: f64,
    /// Weighted overall uncertainty (0.0 = certain, 1.0 = very uncertain)
    pub overall: f64,
}

impl UncertaintyEstimate {
    /// Compute overall uncertainty from component signals
    pub fn compute(
        model_uncertainty: f64,
        retrieval_agreement: f64,
        tool_contradiction: f64,
    ) -> Self {
        // Weights: model uncertainty is the strongest signal
        let overall = 0.2f64
            .mul_add(
                tool_contradiction,
                0.5f64.mul_add(model_uncertainty, 0.3 * (1.0 - retrieval_agreement)),
            )
            .clamp(0.0, 1.0);

        Self {
            model_uncertainty,
            retrieval_agreement,
            tool_contradiction,
            overall,
        }
    }

    /// Create with only model uncertainty (no retrieval or tool data)
    pub fn from_model_only(model_uncertainty: f64) -> Self {
        Self::compute(model_uncertainty, 1.0, 0.0)
    }

    /// Whether uncertainty is high enough to warrant human review
    pub fn should_ask_human(&self, threshold: f64) -> bool {
        self.overall > threshold
    }

    /// Whether uncertainty warrants increased exploration
    pub fn should_explore(&self) -> bool {
        self.model_uncertainty > 0.2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_low_uncertainty() {
        let est = UncertaintyEstimate::compute(0.05, 0.95, 0.0);
        assert!(est.overall < 0.1, "overall={}", est.overall);
        assert!(!est.should_ask_human(0.5));
        assert!(!est.should_explore());
    }

    #[test]
    fn test_high_model_uncertainty() {
        let est = UncertaintyEstimate::compute(0.4, 0.8, 0.0);
        assert!(est.overall > 0.15, "overall={}", est.overall);
        assert!(est.should_explore());
    }

    #[test]
    fn test_retrieval_disagreement() {
        let est = UncertaintyEstimate::compute(0.1, 0.2, 0.0);
        assert!(est.overall > 0.2, "disagreement should raise uncertainty");
    }

    #[test]
    fn test_tool_contradiction() {
        let est = UncertaintyEstimate::compute(0.1, 0.9, 0.8);
        assert!(
            est.overall > 0.15,
            "tool contradiction should raise uncertainty"
        );
    }

    #[test]
    fn test_combined_high_uncertainty() {
        let est = UncertaintyEstimate::compute(0.5, 0.3, 0.7);
        assert!(est.should_ask_human(0.5));
        assert!(est.should_explore());
    }

    #[test]
    fn test_from_model_only() {
        let est = UncertaintyEstimate::from_model_only(0.3);
        assert!(est.retrieval_agreement == 1.0);
        assert!(est.tool_contradiction == 0.0);
        assert!(est.overall > 0.1);
    }

    #[test]
    fn test_overall_clamped() {
        let est = UncertaintyEstimate::compute(1.0, 0.0, 1.0);
        assert!(est.overall <= 1.0);
        assert!(est.overall >= 0.0);
    }
}
