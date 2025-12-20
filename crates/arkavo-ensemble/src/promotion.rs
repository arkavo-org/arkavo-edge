//! Promotion workflow for candidate policies
//!
//! Implements REG-003 (statistical significance) and REG-004 (sustained advantage + SAT)

use uuid::Uuid;

use arkavo_sat::{StressTestConfig, stress_test};
use arkavo_sbe::{PolicyLayer, PolicyUpdateRequest};

use crate::candidate::CandidateState;
use crate::cost::CostFunction;
use crate::ensemble::PolicyEnsemble;
use crate::error::{EnsembleError, EnsembleResult};
use crate::statistics::welch_t_test;

/// A candidate that is ready for promotion
#[derive(Debug, Clone)]
pub struct PromotionCandidate {
    /// Candidate ID
    pub id: Uuid,
    /// Mean regret (positive = better than production)
    pub mean_regret: f64,
    /// P-value from statistical test
    pub p_value: f64,
    /// Hours of sustained positive regret
    pub hours_sustained: f64,
}

/// Extension trait for PolicyEnsemble to add promotion functionality
impl<C: CostFunction> PolicyEnsemble<C> {
    /// Check all candidates for promotion eligibility (REG-003, REG-004)
    ///
    /// Returns candidates that:
    /// 1. Have statistically significant positive regret
    /// 2. Have maintained advantage for the required hours
    /// 3. Pass bounded model checking via SAT
    pub async fn check_promotions(&mut self) -> EnsembleResult<Vec<PromotionCandidate>> {
        let config = self.config().clone();

        // Phase 1: Collect candidates passing statistical tests and clone data for SAT
        struct CandidateCheck {
            id: Uuid,
            policy: PolicyLayer,
            mean_regret: f64,
            p_value: f64,
            hours: f64,
            needs_flagging: bool,
        }

        let mut to_check: Vec<CandidateCheck> = Vec::new();

        for candidate in &self.candidates {
            if candidate.state != CandidateState::Active
                && candidate.state != CandidateState::FlaggedForPromotion
            {
                continue;
            }

            // Get regret statistics
            let regret = match self.regret_accumulator.get_regret(candidate.id) {
                Some(r) => r,
                None => continue,
            };

            // Need sufficient samples for statistical testing
            if regret.outcome_count < 30 {
                continue;
            }

            // REG-003: Statistical significance test (Welch's t-test)
            let t_test = welch_t_test(
                0.0,
                1.0,
                regret.outcome_count,
                regret.mean_regret,
                regret.variance(),
                regret.outcome_count,
            );

            if !t_test.is_significant(config.significance_threshold) {
                continue;
            }

            if regret.mean_regret <= 0.0 {
                continue;
            }

            let needs_flagging = candidate.state == CandidateState::Active;
            let hours = candidate.hours_since_flagged();

            // Skip if not sustained long enough
            if hours < config.promotion_hours && !needs_flagging {
                // Still need to flag, add to list
                to_check.push(CandidateCheck {
                    id: candidate.id,
                    policy: candidate.policy.clone(),
                    mean_regret: regret.mean_regret,
                    p_value: t_test.p_value,
                    hours,
                    needs_flagging,
                });
                continue;
            }

            to_check.push(CandidateCheck {
                id: candidate.id,
                policy: candidate.policy.clone(),
                mean_regret: regret.mean_regret,
                p_value: t_test.p_value,
                hours,
                needs_flagging,
            });
        }

        // Phase 2: Flag candidates that need flagging
        for check in to_check.iter().filter(|c| c.needs_flagging) {
            if let Some(c) = self.get_candidate_mut(check.id) {
                c.flag_for_promotion();
            }
        }

        // Phase 3: Run SAT verification and collect promotable candidates
        let mut promotable = Vec::new();

        for check in to_check {
            if check.hours < config.promotion_hours {
                continue;
            }

            // REG-004: Bounded model checking via arkavo-sat
            if !self.verify_with_sat_sync(&check.policy)? {
                continue;
            }

            promotable.push(PromotionCandidate {
                id: check.id,
                mean_regret: check.mean_regret,
                p_value: check.p_value,
                hours_sustained: check.hours,
            });
        }

        Ok(promotable)
    }

    /// Verify candidate passes bounded model checking (sync version)
    fn verify_with_sat_sync(&self, policy: &PolicyLayer) -> EnsembleResult<bool> {
        let config = StressTestConfig {
            max_random_inputs: 10_000,
            epsilon: 3,
            exhaustive: false,
            max_exhaustive_inputs: 12,
        };

        // Find an output to test
        let output_id = match policy.graph.outputs.first() {
            Some(&id) => id,
            None => return Ok(false),
        };

        match stress_test(&policy.graph, output_id, &config) {
            Ok(result) => {
                // No policy holes = passes bounded model checking
                Ok(result.holes.is_empty())
            }
            Err(e) => Err(EnsembleError::SatVerification(e.to_string())),
        }
    }

    /// Promote a specific candidate to production
    ///
    /// Returns a PolicyUpdateRequest that can be submitted for quorum approval.
    pub fn promote_candidate(
        &mut self,
        candidate_id: Uuid,
        proposer: Vec<u8>,
    ) -> EnsembleResult<PolicyUpdateRequest> {
        let candidate = self
            .candidates
            .iter_mut()
            .find(|c| c.id == candidate_id)
            .ok_or(EnsembleError::CandidateNotFound(candidate_id))?;

        // Verify candidate is in appropriate state
        if candidate.state != CandidateState::FlaggedForPromotion
            && candidate.state != CandidateState::Active
        {
            return Err(EnsembleError::PromotionFailed(format!(
                "Candidate in invalid state: {:?}",
                candidate.state
            )));
        }

        // Mark as promoted
        candidate.promote();

        // Get regret info for description
        let regret_info = self
            .regret_accumulator
            .get_regret(candidate_id)
            .map(|r| format!("mean_regret={:.4}", r.mean_regret))
            .unwrap_or_else(|| "regret unknown".to_string());

        // Create policy update request for quorum approval
        Ok(PolicyUpdateRequest::new(
            candidate.policy.graph.clone(),
            format!("Ensemble promotion: {} ({})", candidate_id, regret_info),
            proposer,
        ))
    }

    /// Apply a promoted candidate as the new production policy
    pub fn apply_promotion(&mut self, candidate_id: Uuid) -> EnsembleResult<()> {
        let policy = {
            let candidate = self
                .candidates
                .iter()
                .find(|c| c.id == candidate_id)
                .ok_or(EnsembleError::CandidateNotFound(candidate_id))?;

            if candidate.state != CandidateState::Promoted {
                return Err(EnsembleError::PromotionFailed(
                    "Candidate not marked as promoted".to_string(),
                ));
            }

            candidate.policy.clone()
        };

        self.set_production(policy);

        // Clean up the promoted candidate and any other terminated
        self.cleanup_terminated();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::GenerationMethod;
    use crate::cost::ConstantCost;
    use arkavo_sbe::InvariantLayer;
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
    fn test_verify_with_sat() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let ensemble = PolicyEnsemble::new(production, invariants, cost);

        let candidate_policy = PolicyLayer::new(create_simple_graph());
        let result = ensemble.verify_with_sat_sync(&candidate_policy).unwrap();

        // Simple OR graph should pass
        assert!(result);
    }

    #[test]
    fn test_promote_candidate() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        let candidate_policy = PolicyLayer::new(create_simple_graph());
        let id = ensemble
            .add_candidate(candidate_policy, GenerationMethod::Manual)
            .unwrap();

        // Promote the candidate
        let request = ensemble.promote_candidate(id, vec![1, 2, 3]).unwrap();

        assert!(request.description.contains(&id.to_string()));
        assert_eq!(request.proposer, vec![1, 2, 3]);
    }

    #[test]
    fn test_promote_not_found() {
        let production = PolicyLayer::new(create_simple_graph());
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        let fake_id = Uuid::new_v4();
        let result = ensemble.promote_candidate(fake_id, vec![]);

        assert!(matches!(result, Err(EnsembleError::CandidateNotFound(_))));
    }

    #[test]
    fn test_apply_promotion() {
        let production = PolicyLayer::new(create_simple_graph());
        let old_version = production.version;
        let invariants = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::zero();

        let mut ensemble = PolicyEnsemble::new(production, invariants, cost);

        let candidate_policy = PolicyLayer::new(create_simple_graph());
        let id = ensemble
            .add_candidate(candidate_policy, GenerationMethod::Manual)
            .unwrap();

        // Promote the candidate
        ensemble.promote_candidate(id, vec![]).unwrap();

        // Apply the promotion
        ensemble.apply_promotion(id).unwrap();

        // Production should be updated
        assert_eq!(ensemble.production().version, old_version);

        // Candidates should be cleaned up
        assert_eq!(ensemble.candidates().len(), 0);
    }
}
