//! Policy ensemble container
//!
//! Manages multiple candidate policies and counterfactual evaluation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use arkavo_sbe::{InvariantLayer, PolicyLayer};

use crate::candidate::{GenerationMethod, PolicyCandidate};
use crate::cost::CostFunction;
use crate::error::{EnsembleError, EnsembleResult};
use crate::regret::{hash_inputs, Outcome, RegretAccumulator, DEFAULT_ATTRIBUTION_WINDOW};
use crate::statistics::DEFAULT_SIGNIFICANCE_THRESHOLD;

/// Default maximum candidates
pub const DEFAULT_MAX_CANDIDATES: usize = 5;

/// Default promotion threshold in hours
pub const DEFAULT_PROMOTION_HOURS: f64 = 24.0;

/// Configuration for the policy ensemble
#[derive(Debug, Clone)]
pub struct EnsembleConfig {
    /// Maximum number of candidates (CFT-001)
    pub max_candidates: usize,
    /// Attribution window for regret calculation (REG-002)
    pub attribution_window: Duration,
    /// Hours of sustained advantage required for promotion (REG-004)
    pub promotion_hours: f64,
    /// Significance threshold for t-test (REG-003)
    pub significance_threshold: f64,
}

impl Default for EnsembleConfig {
    fn default() -> Self {
        Self {
            max_candidates: DEFAULT_MAX_CANDIDATES,
            attribution_window: DEFAULT_ATTRIBUTION_WINDOW,
            promotion_hours: DEFAULT_PROMOTION_HOURS,
            significance_threshold: DEFAULT_SIGNIFICANCE_THRESHOLD,
        }
    }
}

/// Result of a counterfactual evaluation
#[derive(Debug)]
pub struct EvaluationResult {
    /// Production policy output
    pub production_output: HashMap<u16, bool>,
    /// Production cost
    pub production_cost: f64,
    /// Number of candidates evaluated
    pub candidates_evaluated: usize,
    /// Number of candidates discarded
    pub candidates_discarded: usize,
}

/// Policy ensemble container
///
/// Maintains a production policy and up to K candidate policies,
/// evaluating all candidates counterfactually on every real input.
pub struct PolicyEnsemble<C: CostFunction> {
    /// The production policy
    production: PolicyLayer,
    /// Candidate policies being evaluated
    pub(crate) candidates: Vec<PolicyCandidate>,
    /// Invariant layer for constraint checking (CFT-004)
    invariant_layer: Arc<InvariantLayer>,
    /// Regret accumulator for all candidates
    pub(crate) regret_accumulator: RegretAccumulator,
    /// Cost function for regret calculation
    cost_function: C,
    /// Configuration
    config: EnsembleConfig,
}

impl<C: CostFunction> PolicyEnsemble<C> {
    /// Create a new ensemble with the given production policy
    pub fn new(
        production: PolicyLayer,
        invariant_layer: Arc<InvariantLayer>,
        cost_function: C,
    ) -> Self {
        Self::with_config(
            production,
            invariant_layer,
            cost_function,
            EnsembleConfig::default(),
        )
    }

    /// Create with custom configuration
    pub fn with_config(
        production: PolicyLayer,
        invariant_layer: Arc<InvariantLayer>,
        cost_function: C,
        config: EnsembleConfig,
    ) -> Self {
        let regret_accumulator =
            RegretAccumulator::with_config(config.attribution_window, 10_000);

        Self {
            production,
            candidates: Vec::with_capacity(config.max_candidates),
            invariant_layer,
            regret_accumulator,
            cost_function,
            config,
        }
    }

    /// Get the production policy
    pub fn production(&self) -> &PolicyLayer {
        &self.production
    }

    /// Get all candidates
    pub fn candidates(&self) -> &[PolicyCandidate] {
        &self.candidates
    }

    /// Get a specific candidate
    pub fn get_candidate(&self, id: Uuid) -> Option<&PolicyCandidate> {
        self.candidates.iter().find(|c| c.id == id)
    }

    /// Get mutable access to a candidate
    pub fn get_candidate_mut(&mut self, id: Uuid) -> Option<&mut PolicyCandidate> {
        self.candidates.iter_mut().find(|c| c.id == id)
    }

    /// Get regret accumulator
    pub fn regret_accumulator(&self) -> &RegretAccumulator {
        &self.regret_accumulator
    }

    /// Get configuration
    pub fn config(&self) -> &EnsembleConfig {
        &self.config
    }

    /// Add a new candidate (CFT-001)
    pub fn add_candidate(
        &mut self,
        policy: PolicyLayer,
        method: GenerationMethod,
    ) -> EnsembleResult<Uuid> {
        // Check capacity
        let active_count = self
            .candidates
            .iter()
            .filter(|c| !c.state.is_terminal())
            .count();

        if active_count >= self.config.max_candidates {
            return Err(EnsembleError::MaxCandidatesReached {
                max: self.config.max_candidates,
            });
        }

        let mut candidate = PolicyCandidate::new(policy, method);
        candidate.activate();
        let id = candidate.id;

        self.candidates.push(candidate);
        Ok(id)
    }

    /// Discard a candidate
    pub fn discard_candidate(&mut self, id: Uuid) {
        if let Some(candidate) = self.get_candidate_mut(id) {
            candidate.discard();
        }
        self.regret_accumulator.remove_candidate(id);
    }

    /// Evaluate counterfactually on input (CFT-003, CFT-004)
    ///
    /// Evaluates all active candidates on the given input and records
    /// regret. Candidates violating invariants are immediately discarded.
    pub fn evaluate_counterfactual(
        &mut self,
        inputs: &HashMap<u16, bool>,
    ) -> EnsembleResult<EvaluationResult> {
        // 1. Evaluate production policy
        let production_output = torg_core::evaluate(&self.production.graph, inputs)
            .map_err(|e| EnsembleError::Evaluation(e.to_string()))?;
        let production_cost = self.cost_function.compute(inputs, &production_output);

        // 2. Evaluate all active candidates
        let mut candidate_costs = HashMap::new();
        let mut to_discard = Vec::new();
        let mut evaluated = 0;

        for candidate in &mut self.candidates {
            if !candidate.state.is_evaluable() {
                continue;
            }

            // CFT-004: Pre-check invariants on input
            if self.invariant_layer.check_violation(inputs).is_some() {
                to_discard.push(candidate.id);
                candidate.record_violation();
                continue;
            }

            // Sub-microsecond TØRG evaluation
            match torg_core::evaluate(&candidate.policy.graph, inputs) {
                Ok(outputs) => {
                    // Post-check: verify output doesn't violate invariants
                    let mut violation = false;
                    for contract in self.invariant_layer.contracts() {
                        if let Ok(false) = contract.evaluate(&outputs) {
                            violation = true;
                            break;
                        }
                    }

                    if violation {
                        to_discard.push(candidate.id);
                        candidate.record_violation();
                    } else {
                        let cost = self.cost_function.compute(inputs, &outputs);
                        candidate_costs.insert(candidate.id, cost);
                        candidate.record_evaluation();
                        evaluated += 1;
                    }
                }
                Err(_) => {
                    to_discard.push(candidate.id);
                }
            }
        }

        // 3. Discard violating candidates immediately (CFT-004)
        let discarded = to_discard.len();
        for id in to_discard {
            self.discard_candidate(id);
        }

        // 4. Record outcome for regret tracking (REG-001, REG-002)
        self.regret_accumulator
            .record_outcome(Outcome {
                timestamp: std::time::Instant::now(),
                input_hash: hash_inputs(inputs),
                production_cost,
                candidate_costs,
            });

        Ok(EvaluationResult {
            production_output,
            production_cost,
            candidates_evaluated: evaluated,
            candidates_discarded: discarded,
        })
    }

    /// Set new production policy (after promotion)
    pub fn set_production(&mut self, policy: PolicyLayer) {
        self.production = policy;
    }

    /// Clean up terminated candidates
    pub fn cleanup_terminated(&mut self) {
        self.candidates.retain(|c| !c.state.is_terminal());
    }

    /// Get count of active candidates
    pub fn active_candidate_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|c| c.state.is_evaluable())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidateState;
    use crate::cost::ConstantCost;
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
    fn test_ensemble_creation() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let ensemble = PolicyEnsemble::new(production, invariants, cost);

        assert_eq!(ensemble.candidates().len(), 0);
        assert_eq!(ensemble.config().max_candidates, DEFAULT_MAX_CANDIDATES);
    }

    #[test]
    fn test_add_candidate() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        let candidate_policy = PolicyLayer::new(create_simple_graph());
        let id = ensemble
            .add_candidate(candidate_policy, GenerationMethod::Manual)
            .unwrap();

        assert_eq!(ensemble.candidates().len(), 1);
        assert!(ensemble.get_candidate(id).is_some());
        assert_eq!(
            ensemble.get_candidate(id).unwrap().state,
            CandidateState::Active
        );
    }

    #[test]
    fn test_max_candidates() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let config = EnsembleConfig {
            max_candidates: 2,
            ..Default::default()
        };
        let mut ensemble =
            PolicyEnsemble::with_config(production, invariants, cost, config);

        // Add 2 candidates - should succeed
        for _ in 0..2 {
            let policy = PolicyLayer::new(create_simple_graph());
            ensemble
                .add_candidate(policy, GenerationMethod::Manual)
                .unwrap();
        }

        // Add 3rd candidate - should fail
        let policy = PolicyLayer::new(create_simple_graph());
        let result = ensemble.add_candidate(policy, GenerationMethod::Manual);
        assert!(matches!(
            result,
            Err(EnsembleError::MaxCandidatesReached { .. })
        ));
    }

    #[test]
    fn test_evaluate_counterfactual() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::new(1.0);

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        // Add a candidate
        let policy = PolicyLayer::new(create_simple_graph());
        let id = ensemble
            .add_candidate(policy, GenerationMethod::Manual)
            .unwrap();

        // Evaluate
        let mut inputs = HashMap::new();
        inputs.insert(0, true);

        let result = ensemble.evaluate_counterfactual(&inputs).unwrap();

        assert_eq!(result.candidates_evaluated, 1);
        assert_eq!(result.candidates_discarded, 0);

        // Check regret was recorded
        let regret = ensemble.regret_accumulator().get_regret(id).unwrap();
        assert_eq!(regret.outcome_count, 1);
    }

    #[test]
    fn test_discard_candidate() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        let policy = PolicyLayer::new(create_simple_graph());
        let id = ensemble
            .add_candidate(policy, GenerationMethod::Manual)
            .unwrap();

        ensemble.discard_candidate(id);

        assert_eq!(
            ensemble.get_candidate(id).unwrap().state,
            CandidateState::Discarded
        );
        assert_eq!(ensemble.active_candidate_count(), 0);
    }

    #[test]
    fn test_cleanup_terminated() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        let policy = PolicyLayer::new(create_simple_graph());
        let id = ensemble
            .add_candidate(policy, GenerationMethod::Manual)
            .unwrap();

        ensemble.discard_candidate(id);
        assert_eq!(ensemble.candidates().len(), 1);

        ensemble.cleanup_terminated();
        assert_eq!(ensemble.candidates().len(), 0);
    }
}
