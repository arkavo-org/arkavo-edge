//! Contract negotiation state machine
//!
//! Manages the pre-execution contract protocol between Operator and Critic.
//! Contracts are proposed, reviewed (max 2 rounds), then approved or escalated.

use std::collections::HashMap;

use arkavo_protocol::task_contract::{
    AcceptanceCriterion, ApprovedContract, ContractProposal, ContractReview, CriterionFeedback,
    MAX_REVISION_ROUNDS, VerificationMethod,
};
use uuid::Uuid;

/// Actions the negotiator can take after processing a review
#[derive(Debug, Clone)]
pub(super) enum ContractAction {
    /// Contract approved, ready for execution
    Approved(ApprovedContract),
    /// Operator should revise and resubmit
    RequestRevision(ContractReview),
    /// Negotiation failed, escalate to Planner for task reformulation
    Escalate { contract_id: Uuid, reason: String },
}

/// Internal negotiation state
#[derive(Debug, Clone)]
enum NegotiationState {
    Proposed(ContractProposal),
    Approved(ApprovedContract),
    Escalated { reason: String },
}

/// Manages contract negotiation lifecycle
pub(super) struct ContractNegotiator {
    max_rounds: u8,
    active: HashMap<Uuid, NegotiationState>,
}

impl ContractNegotiator {
    pub(super) fn new() -> Self {
        Self {
            max_rounds: MAX_REVISION_ROUNDS,
            active: HashMap::new(),
        }
    }

    /// Extract a contract proposal from task content using heuristics.
    ///
    /// Parses goal and acceptance criteria from structured text.
    /// Keywords like "must", "should", "ensure" indicate criteria.
    pub(super) fn propose(&mut self, task_id: &str, task_content: &str) -> ContractProposal {
        let (goal, criteria) = extract_goal_and_criteria(task_content);
        let proposal = ContractProposal::new(task_id.to_string(), goal, criteria);
        self.active.insert(
            proposal.contract_id,
            NegotiationState::Proposed(proposal.clone()),
        );
        proposal
    }

    /// Review a proposal: validate criteria are testable and non-contradictory.
    ///
    /// Returns a review with per-criterion feedback. Approval happens when
    /// all criteria are accepted; otherwise requests revision.
    pub(super) fn review(&self, proposal: &ContractProposal) -> ContractReview {
        let mut feedback = Vec::new();
        let mut all_accepted = true;

        for criterion in &proposal.acceptance_criteria {
            let (accepted, reason) = validate_criterion(criterion);
            if !accepted {
                all_accepted = false;
            }
            feedback.push(CriterionFeedback {
                criterion_id: criterion.id.clone(),
                accepted,
                reason,
                suggested_revision: None,
            });
        }

        // Empty criteria is suspicious — request at least one
        if proposal.acceptance_criteria.is_empty() {
            all_accepted = false;
        }

        if all_accepted {
            ContractReview::approve(proposal.contract_id, 1)
        } else {
            ContractReview::revise(proposal.contract_id, feedback, 1)
        }
    }

    /// Advance the negotiation state machine based on a review.
    ///
    /// - If approved: transition to Approved state
    /// - If not approved and rounds remain: request revision
    /// - If not approved and rounds exhausted: escalate to Planner
    pub(super) fn advance(&mut self, contract_id: Uuid, review: &ContractReview) -> ContractAction {
        let state = match self.active.get(&contract_id) {
            Some(state) => state.clone(),
            None => {
                return ContractAction::Escalate {
                    contract_id,
                    reason: "contract not found".to_string(),
                };
            }
        };

        let proposal = match &state {
            NegotiationState::Proposed(p) => p,
            NegotiationState::Approved(c) => {
                return ContractAction::Approved(c.clone());
            }
            NegotiationState::Escalated { reason } => {
                return ContractAction::Escalate {
                    contract_id,
                    reason: reason.clone(),
                };
            }
        };

        if review.approved {
            let contract = ApprovedContract::from_proposal(proposal, review.round);
            self.active
                .insert(contract_id, NegotiationState::Approved(contract.clone()));
            ContractAction::Approved(contract)
        } else if review.round >= self.max_rounds {
            let reason = format!("failed to converge after {} rounds", self.max_rounds);
            self.active.insert(
                contract_id,
                NegotiationState::Escalated {
                    reason: reason.clone(),
                },
            );
            ContractAction::Escalate {
                contract_id,
                reason,
            }
        } else {
            // Build revision review for next round
            let next_review =
                ContractReview::revise(contract_id, review.feedback.clone(), review.round + 1);
            ContractAction::RequestRevision(next_review)
        }
    }

    /// Remove completed contracts to prevent unbounded growth
    pub(super) fn cleanup(&mut self, contract_id: Uuid) {
        self.active.remove(&contract_id);
    }
}

impl Default for ContractNegotiator {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract goal and acceptance criteria from task text using heuristics
fn extract_goal_and_criteria(content: &str) -> (String, Vec<AcceptanceCriterion>) {
    let lines: Vec<&str> = content.lines().collect();
    let mut criteria = Vec::new();
    let mut goal_lines = Vec::new();
    let mut criterion_counter = 0;

    // Criterion indicator keywords (case-insensitive matching done below)
    let criterion_keywords = ["must", "should", "ensure", "verify", "require"];

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let lower = trimmed.to_lowercase();

        // Detect numbered list items (e.g., "1. ", "2) ") or bullet points
        let is_list_item = trimmed.starts_with(|c: char| c.is_ascii_digit())
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ");

        let has_keyword = criterion_keywords.iter().any(|kw| lower.contains(kw));

        if is_list_item || has_keyword {
            criterion_counter += 1;
            // Strip list prefix
            let text = trimmed
                .trim_start_matches(|c: char| {
                    c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*'
                })
                .trim();
            criteria.push(AcceptanceCriterion {
                id: format!("ac{criterion_counter}"),
                criterion: text.to_string(),
                verification: VerificationMethod::OutputMatch,
                threshold: None,
            });
        } else {
            goal_lines.push(trimmed);
        }
    }

    let goal = if goal_lines.is_empty() {
        content.chars().take(200).collect::<String>()
    } else {
        goal_lines.join(" ")
    };

    (goal, criteria)
}

/// Validate a single criterion: is it testable and well-formed?
fn validate_criterion(criterion: &AcceptanceCriterion) -> (bool, Option<String>) {
    // Criterion must have non-empty text
    if criterion.criterion.trim().is_empty() {
        return (false, Some("criterion text is empty".to_string()));
    }

    // Criterion must be of reasonable length (not too vague)
    if criterion.criterion.len() < 5 {
        return (
            false,
            Some("criterion too vague — provide more detail".to_string()),
        );
    }

    // StateDelta verification should have a threshold
    if matches!(criterion.verification, VerificationMethod::StateDelta)
        && criterion.threshold.is_none()
    {
        return (
            false,
            Some("state_delta verification requires a threshold expression".to_string()),
        );
    }

    (true, None)
}

/// Convert an ApprovedContract to JSON context for VerificationInput injection
pub(super) fn contract_to_verification_context(contract: &ApprovedContract) -> serde_json::Value {
    let criteria: Vec<serde_json::Value> = contract
        .acceptance_criteria
        .iter()
        .map(|ac| {
            serde_json::json!({
                "id": ac.id,
                "criterion": ac.criterion,
                "threshold": ac.threshold,
            })
        })
        .collect();

    serde_json::json!({
        "contract_id": contract.contract_id.to_string(),
        "goal": contract.goal,
        "acceptance_criteria": criteria,
        "rubric_weights": {
            "goal_fidelity": contract.rubric_weights.goal_fidelity,
            "efficiency": contract.rubric_weights.efficiency,
            "state_coherence": contract.rubric_weights.state_coherence,
            "adaptability": contract.rubric_weights.adaptability,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_propose_extracts_goal() {
        let mut negotiator = ContractNegotiator::new();
        let proposal = negotiator.propose(
            "task-001",
            "Place a rice growing zone in the colony.\n\
             1. Zone must cover at least 6x6 tiles\n\
             2. Zone should be on fertile soil",
        );
        assert_eq!(proposal.task_id, "task-001");
        assert!(!proposal.goal.is_empty());
        assert_eq!(proposal.acceptance_criteria.len(), 2);
    }

    #[test]
    fn test_propose_with_keywords() {
        let mut negotiator = ContractNegotiator::new();
        let proposal = negotiator.propose(
            "task-002",
            "Build a defensive wall.\n\
             Ensure the wall covers all approach vectors.\n\
             Must use stone blocks for durability.",
        );
        assert_eq!(proposal.acceptance_criteria.len(), 2);
    }

    #[test]
    fn test_propose_no_criteria() {
        let mut negotiator = ContractNegotiator::new();
        let proposal = negotiator.propose("task-003", "Do something vague");
        assert!(proposal.acceptance_criteria.is_empty());
        assert!(!proposal.goal.is_empty());
    }

    #[test]
    fn test_review_approves_valid_criteria() {
        let negotiator = ContractNegotiator::new();
        let proposal = ContractProposal::new(
            "task-001".to_string(),
            "Place zone".to_string(),
            vec![AcceptanceCriterion {
                id: "ac1".to_string(),
                criterion: "Zone covers minimum 6x6 tiles".to_string(),
                verification: VerificationMethod::OutputMatch,
                threshold: None,
            }],
        );
        let review = negotiator.review(&proposal);
        assert!(review.approved);
    }

    #[test]
    fn test_review_rejects_empty_criteria() {
        let negotiator = ContractNegotiator::new();
        let proposal = ContractProposal::new("task-001".to_string(), "Goal".to_string(), vec![]);
        let review = negotiator.review(&proposal);
        assert!(!review.approved);
    }

    #[test]
    fn test_review_rejects_state_delta_without_threshold() {
        let negotiator = ContractNegotiator::new();
        let proposal = ContractProposal::new(
            "task-001".to_string(),
            "Goal".to_string(),
            vec![AcceptanceCriterion {
                id: "ac1".to_string(),
                criterion: "Zone covers area".to_string(),
                verification: VerificationMethod::StateDelta,
                threshold: None, // missing!
            }],
        );
        let review = negotiator.review(&proposal);
        assert!(!review.approved);
    }

    #[test]
    fn test_advance_approve() {
        let mut negotiator = ContractNegotiator::new();
        let proposal = negotiator.propose("task-001", "Build wall.\n1. Wall covers north side");
        let review = ContractReview::approve(proposal.contract_id, 1);
        let action = negotiator.advance(proposal.contract_id, &review);
        assert!(matches!(action, ContractAction::Approved(_)));
    }

    #[test]
    fn test_advance_revision_then_approve() {
        let mut negotiator = ContractNegotiator::new();
        let proposal = negotiator.propose("task-001", "Build wall.\n1. Wall covers north side");

        // Round 1: revision requested
        let review = ContractReview::revise(proposal.contract_id, vec![], 1);
        let action = negotiator.advance(proposal.contract_id, &review);
        assert!(matches!(action, ContractAction::RequestRevision(_)));

        // Round 2: approved
        let review = ContractReview::approve(proposal.contract_id, 2);
        let action = negotiator.advance(proposal.contract_id, &review);
        assert!(matches!(action, ContractAction::Approved(_)));
    }

    #[test]
    fn test_advance_escalation_after_max_rounds() {
        let mut negotiator = ContractNegotiator::new();
        let proposal = negotiator.propose("task-001", "Build wall.\n1. Wall covers north side");

        // Round 2 (max): still not approved → escalate
        let review = ContractReview::revise(proposal.contract_id, vec![], 2);
        let action = negotiator.advance(proposal.contract_id, &review);
        assert!(matches!(action, ContractAction::Escalate { .. }));
    }

    #[test]
    fn test_cleanup_removes_contract() {
        let mut negotiator = ContractNegotiator::new();
        let proposal = negotiator.propose("task-001", "Goal.\n1. Test criterion");
        negotiator.cleanup(proposal.contract_id);
        // After cleanup, advance should fail (contract not found)
        let review = ContractReview::approve(proposal.contract_id, 1);
        let action = negotiator.advance(proposal.contract_id, &review);
        assert!(matches!(action, ContractAction::Escalate { .. }));
    }

    #[test]
    fn test_contract_to_verification_context() {
        let proposal = ContractProposal::new(
            "task-001".to_string(),
            "Place zone".to_string(),
            vec![AcceptanceCriterion {
                id: "ac1".to_string(),
                criterion: "covers area".to_string(),
                verification: VerificationMethod::OutputMatch,
                threshold: Some("area >= 36".to_string()),
            }],
        );
        let contract = ApprovedContract::from_proposal(&proposal, 1);
        let ctx = contract_to_verification_context(&contract);
        assert!(ctx.get("acceptance_criteria").unwrap().is_array());
        assert!(ctx.get("rubric_weights").is_some());
    }

    #[test]
    fn test_extract_goal_and_criteria_bullets() {
        let (goal, criteria) = extract_goal_and_criteria(
            "Deploy the application.\n- Must pass all tests\n- Should have zero downtime",
        );
        assert!(!goal.is_empty());
        assert_eq!(criteria.len(), 2);
    }

    #[test]
    fn test_validate_criterion_empty() {
        let c = AcceptanceCriterion {
            id: "ac1".to_string(),
            criterion: "".to_string(),
            verification: VerificationMethod::OutputMatch,
            threshold: None,
        };
        let (accepted, reason) = validate_criterion(&c);
        assert!(!accepted);
        assert!(reason.is_some());
    }

    #[test]
    fn test_validate_criterion_too_short() {
        let c = AcceptanceCriterion {
            id: "ac1".to_string(),
            criterion: "ok".to_string(),
            verification: VerificationMethod::OutputMatch,
            threshold: None,
        };
        let (accepted, _) = validate_criterion(&c);
        assert!(!accepted);
    }
}
