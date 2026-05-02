//! Task Contract Protocol types
//!
//! Pre-execution contracts between Operator and Critic that define
//! deliverables, acceptance criteria, and verification methods.
//! Contracts flow via `MessagePart::Data` with schema URNs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Schema URN for contract proposal messages
pub const SCHEMA_CONTRACT_PROPOSAL: &str = "urn:arkavo:contract:proposal";
/// Schema URN for contract review messages
pub const SCHEMA_CONTRACT_REVIEW: &str = "urn:arkavo:contract:review";
/// Schema URN for approved contract messages
pub const SCHEMA_CONTRACT_APPROVED: &str = "urn:arkavo:contract:approved";

/// Maximum revision rounds before escalation to Planner
pub const MAX_REVISION_ROUNDS: u8 = 2;

/// Weights for the four rubric dimensions
///
/// Defined here (not in arkavo-critic) to avoid reverse dependency.
/// Sum must equal 1.0.
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
    pub fn is_valid(&self) -> bool {
        let sum = self.goal_fidelity + self.efficiency + self.state_coherence + self.adaptability;
        (sum - 1.0).abs() < 0.01
    }
}

/// How an acceptance criterion is verified
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMethod {
    /// Compare state before/after via observation deltas
    StateDelta,
    /// Match against expected output pattern
    OutputMatch,
    /// Custom verification logic (description of method)
    Custom(String),
}

/// A single acceptance criterion within a contract
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    /// Unique criterion identifier (e.g., "ac1")
    pub id: String,
    /// Human-readable description of what must be true
    pub criterion: String,
    /// How to verify this criterion
    pub verification: VerificationMethod,
    /// Optional threshold expression (e.g., "zone.area >= 36")
    #[serde(default)]
    pub threshold: Option<String>,
}

/// Operator's proposed contract before execution begins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractProposal {
    /// Unique contract identifier
    pub contract_id: Uuid,
    /// Associated task identifier
    pub task_id: String,
    /// The operator's role in the swarm
    #[serde(default)]
    pub operator_role: Option<String>,
    /// What the operator intends to achieve
    pub goal: String,
    /// Measurable acceptance criteria
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    /// Estimated number of execution steps
    #[serde(default)]
    pub estimated_steps: u32,
    /// Optional rubric weight overrides
    #[serde(default)]
    pub rubric_weights: Option<RubricWeights>,
}

impl ContractProposal {
    pub fn new(task_id: String, goal: String, criteria: Vec<AcceptanceCriterion>) -> Self {
        Self {
            contract_id: Uuid::new_v4(),
            task_id,
            operator_role: None,
            goal,
            acceptance_criteria: criteria,
            estimated_steps: 1,
            rubric_weights: None,
        }
    }
}

/// Feedback on a single criterion during review
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionFeedback {
    /// Which criterion this feedback is for
    pub criterion_id: String,
    /// Whether this criterion was accepted
    pub accepted: bool,
    /// Reason for revision (if not accepted)
    #[serde(default)]
    pub reason: Option<String>,
    /// Suggested revision text
    #[serde(default)]
    pub suggested_revision: Option<String>,
}

/// Critic's review of a contract proposal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractReview {
    /// Which contract this reviews
    pub contract_id: Uuid,
    /// Overall approval
    pub approved: bool,
    /// Per-criterion feedback
    pub feedback: Vec<CriterionFeedback>,
    /// Current revision round (1-based)
    pub round: u8,
    /// Maximum allowed rounds
    pub max_rounds: u8,
}

impl ContractReview {
    pub fn approve(contract_id: Uuid, round: u8) -> Self {
        Self {
            contract_id,
            approved: true,
            feedback: vec![],
            round,
            max_rounds: MAX_REVISION_ROUNDS,
        }
    }

    pub fn revise(contract_id: Uuid, feedback: Vec<CriterionFeedback>, round: u8) -> Self {
        Self {
            contract_id,
            approved: false,
            feedback,
            round,
            max_rounds: MAX_REVISION_ROUNDS,
        }
    }
}

/// Maturity tier for approved contracts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractTier {
    /// Initial approval; not yet validated by execution
    Candidate,
    /// Promoted after task evaluation passes
    Canonical,
}

/// A fully approved contract ready for execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedContract {
    /// Contract identifier
    pub contract_id: Uuid,
    /// Associated task identifier
    pub task_id: String,
    /// Agreed-upon goal
    pub goal: String,
    /// Finalized acceptance criteria
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    /// Rubric weights (defaults if not overridden)
    pub rubric_weights: RubricWeights,
    /// When the contract was approved
    pub approved_at: DateTime<Utc>,
    /// Maturity tier
    pub tier: ContractTier,
    /// How many revision rounds it took (1 = first round approval)
    pub revision_rounds: u8,
}

impl ApprovedContract {
    pub fn from_proposal(proposal: &ContractProposal, revision_rounds: u8) -> Self {
        Self {
            contract_id: proposal.contract_id,
            task_id: proposal.task_id.clone(),
            goal: proposal.goal.clone(),
            acceptance_criteria: proposal.acceptance_criteria.clone(),
            rubric_weights: proposal.rubric_weights.clone().unwrap_or_default(),
            approved_at: Utc::now(),
            tier: ContractTier::Candidate,
            revision_rounds,
        }
    }

    /// Promote from Candidate to Canonical after evaluation passes
    pub fn promote(&mut self) {
        self.tier = ContractTier::Canonical;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_weights() {
        let w = RubricWeights::default();
        assert!(w.is_valid());
    }

    #[test]
    fn test_proposal_creation() {
        let criteria = vec![AcceptanceCriterion {
            id: "ac1".to_string(),
            criterion: "zone covers area".to_string(),
            verification: VerificationMethod::StateDelta,
            threshold: Some("zone.area >= 36".to_string()),
        }];
        let proposal = ContractProposal::new(
            "task-001".to_string(),
            "Place rice growing zone".to_string(),
            criteria,
        );
        assert_eq!(proposal.task_id, "task-001");
        assert_eq!(proposal.acceptance_criteria.len(), 1);
        assert!(proposal.rubric_weights.is_none());
    }

    #[test]
    fn test_review_approve() {
        let id = Uuid::new_v4();
        let review = ContractReview::approve(id, 1);
        assert!(review.approved);
        assert_eq!(review.round, 1);
        assert!(review.feedback.is_empty());
    }

    #[test]
    fn test_review_revise() {
        let id = Uuid::new_v4();
        let feedback = vec![CriterionFeedback {
            criterion_id: "ac1".to_string(),
            accepted: false,
            reason: Some("threshold too generous".to_string()),
            suggested_revision: Some("zone.path_distance <= 15".to_string()),
        }];
        let review = ContractReview::revise(id, feedback, 1);
        assert!(!review.approved);
        assert_eq!(review.feedback.len(), 1);
    }

    #[test]
    fn test_approved_contract_from_proposal() {
        let proposal =
            ContractProposal::new("task-001".to_string(), "Place zone".to_string(), vec![]);
        let contract = ApprovedContract::from_proposal(&proposal, 1);
        assert_eq!(contract.tier, ContractTier::Candidate);
        assert_eq!(contract.revision_rounds, 1);
        assert!(contract.rubric_weights.is_valid());
    }

    #[test]
    fn test_contract_promotion() {
        let proposal = ContractProposal::new("t1".to_string(), "goal".to_string(), vec![]);
        let mut contract = ApprovedContract::from_proposal(&proposal, 1);
        assert_eq!(contract.tier, ContractTier::Candidate);
        contract.promote();
        assert_eq!(contract.tier, ContractTier::Canonical);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let proposal = ContractProposal::new(
            "task-001".to_string(),
            "Place zone".to_string(),
            vec![AcceptanceCriterion {
                id: "ac1".to_string(),
                criterion: "test".to_string(),
                verification: VerificationMethod::Custom("manual check".to_string()),
                threshold: None,
            }],
        );
        let json = serde_json::to_string(&proposal).unwrap();
        let parsed: ContractProposal = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.task_id, "task-001");
        assert_eq!(parsed.acceptance_criteria.len(), 1);
    }

    #[test]
    fn test_schema_urns_are_distinct() {
        assert_ne!(SCHEMA_CONTRACT_PROPOSAL, SCHEMA_CONTRACT_REVIEW);
        assert_ne!(SCHEMA_CONTRACT_REVIEW, SCHEMA_CONTRACT_APPROVED);
        assert_ne!(SCHEMA_CONTRACT_PROPOSAL, SCHEMA_CONTRACT_APPROVED);
    }
}
