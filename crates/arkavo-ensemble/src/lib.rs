//! Policy ensemble with counterfactual evaluation for TØRG
//!
//! This crate implements Phase 3 of Symbolic Boundary Evolution (SBE):
//! counterfactual evaluation of candidate policies against production.
//!
//! # Overview
//!
//! The ensemble maintains up to K candidate policies (default 5) that are
//! evaluated counterfactually on every real input. Candidates that demonstrate
//! statistically significant improvement over the production policy are
//! flagged for promotion after sustaining their advantage.
//!
//! # Key Components
//!
//! - **PolicyEnsemble**: Container managing production and candidate policies
//! - **CostFunction**: Trait for defining how to measure policy quality
//! - **RegretAccumulator**: Tracks cumulative regret with attribution windows
//! - **PromotionCandidate**: Workflow for promoting candidates to production
//!
//! # Requirements Implemented
//!
//! - CFT-001: Maintain up to K candidate policy variants
//! - CFT-002: Candidates via LLM synthesis, boundary mutation, anomaly remediation
//! - CFT-003: Evaluate all candidates on every real input (sub-microsecond)
//! - CFT-004: Discard candidates violating Invariant layer immediately
//! - REG-001: Regret = sum of (actual_cost - counterfactual_cost)
//! - REG-002: Configurable outcome attribution windows (default 1 second)
//! - REG-003: Flag candidates with statistically significant positive regret
//! - REG-004: Promotion requires 24h sustained advantage + bounded model checking
//!
//! # Example
//!
//! ```ignore
//! use arkavo_ensemble::{PolicyEnsemble, EnsembleConfig, BinaryCost};
//! use arkavo_sbe::{PolicyLayer, InvariantLayer};
//! use std::sync::Arc;
//!
//! // Create ensemble with production policy
//! let production = PolicyLayer::new(graph);
//! let invariants = Arc::new(InvariantLayer::new());
//! let cost = BinaryCost::single(output_id, true);
//!
//! let mut ensemble = PolicyEnsemble::new(production, invariants, cost);
//!
//! // Add a candidate
//! let candidate_id = ensemble.add_candidate(candidate_policy, GenerationMethod::Manual)?;
//!
//! // Evaluate counterfactually on real input
//! let result = ensemble.evaluate_counterfactual(&inputs)?;
//!
//! // Check for promotable candidates
//! let promotable = ensemble.check_promotions().await?;
//! ```

pub mod candidate;
pub mod cost;
pub mod ensemble;
pub mod error;
pub mod generator;
pub mod promotion;
pub mod regret;
pub mod statistics;
pub mod storage;

// Re-export main types
pub use candidate::{CandidateState, GenerationMethod, PolicyCandidate};
pub use cost::{BinaryCost, BoundaryDistanceCost, ConstantCost, CostFunction, WeightedCost};
pub use ensemble::{
    DEFAULT_MAX_CANDIDATES, DEFAULT_PROMOTION_HOURS, EnsembleConfig, EvaluationResult,
    PolicyEnsemble,
};
pub use error::{EnsembleError, EnsembleResult};
pub use generator::{
    LlmSynthesizer, SynthesisContext, SynthesisError, generate_anomaly_remediation,
    generate_boundary_mutation, generate_llm_candidate,
};
pub use promotion::PromotionCandidate;
pub use regret::{
    CandidateRegret, DEFAULT_ATTRIBUTION_WINDOW, DEFAULT_MAX_OUTCOMES, Outcome, RegretAccumulator,
    hash_inputs,
};
pub use statistics::{DEFAULT_SIGNIFICANCE_THRESHOLD, TTestResult, welch_t_test};
pub use storage::{SqliteEnsembleStore, StoredCandidate, StoredRegret};

// Re-export dependencies for convenience
pub use arkavo_sat;
pub use arkavo_sbe::{InvariantLayer, PolicyLayer};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
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

    #[test]
    fn test_full_workflow() {
        // Create ensemble with production policy
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::new(1.0);

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        // Add a candidate with lower cost
        let candidate_policy = PolicyLayer::new(create_simple_graph());
        let id = ensemble
            .add_candidate(candidate_policy, GenerationMethod::Manual)
            .unwrap();

        // Evaluate counterfactually
        let mut inputs = HashMap::new();
        inputs.insert(0, true);

        for _ in 0..10 {
            let result = ensemble.evaluate_counterfactual(&inputs).unwrap();
            assert_eq!(result.candidates_evaluated, 1);
        }

        // Check regret was accumulated
        let regret = ensemble.regret_accumulator().get_regret(id).unwrap();
        assert_eq!(regret.outcome_count, 10);
    }

    #[test]
    fn test_max_candidates_enforcement() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let config = EnsembleConfig {
            max_candidates: 2,
            ..Default::default()
        };
        let mut ensemble = PolicyEnsemble::with_config(production, invariants, cost, config);

        // Add 2 candidates
        for _ in 0..2 {
            let policy = PolicyLayer::new(create_simple_graph());
            ensemble
                .add_candidate(policy, GenerationMethod::Manual)
                .unwrap();
        }

        // Third should fail
        let policy = PolicyLayer::new(create_simple_graph());
        assert!(
            ensemble
                .add_candidate(policy, GenerationMethod::Manual)
                .is_err()
        );
    }

    #[test]
    fn test_regret_calculation() {
        let mut acc = RegretAccumulator::new();
        let id = uuid::Uuid::new_v4();

        let mut costs = HashMap::new();
        costs.insert(id, 0.5); // Candidate has lower cost

        // Record outcomes where candidate is better
        for _ in 0..5 {
            acc.record_outcome(Outcome {
                timestamp: std::time::Instant::now(),
                input_hash: 12345,
                production_cost: 1.0,
                candidate_costs: costs.clone(),
            });
        }

        let regret = acc.get_regret(id).unwrap();
        assert_eq!(regret.outcome_count, 5);
        assert!(regret.mean_regret > 0.0); // Positive = candidate is better
        assert!((regret.mean_regret - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_welch_t_test() {
        // Test with known values
        let result = welch_t_test(0.0, 1.0, 100, 0.5, 1.0, 100);

        // Means are different, should be significant
        assert!(result.t_statistic.abs() > 1.0);
        assert!(result.degrees_of_freedom > 0.0);
        assert!(result.p_value < 0.05);
        assert!(result.is_significant_default());
    }

    #[test]
    fn test_binary_cost() {
        let cost = BinaryCost::single(1, true);

        let mut outputs_correct = HashMap::new();
        outputs_correct.insert(1, true);
        assert!((cost.compute(&HashMap::new(), &outputs_correct) - 0.0).abs() < f64::EPSILON);

        let mut outputs_wrong = HashMap::new();
        outputs_wrong.insert(1, false);
        assert!((cost.compute(&HashMap::new(), &outputs_wrong) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_candidate_lifecycle() {
        let policy = PolicyLayer::new(create_simple_graph());
        let mut candidate = PolicyCandidate::new(policy, GenerationMethod::Manual);

        assert_eq!(candidate.state, CandidateState::Pending);

        candidate.activate();
        assert_eq!(candidate.state, CandidateState::Active);

        candidate.flag_for_promotion();
        assert_eq!(candidate.state, CandidateState::FlaggedForPromotion);
        assert!(candidate.first_flagged_at.is_some());

        candidate.promote();
        assert_eq!(candidate.state, CandidateState::Promoted);
    }
}
