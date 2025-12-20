//! Policy candidate types and lifecycle management
//!
//! Candidates progress through states: Pending → Active → FlaggedForPromotion → Promoted/Discarded

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use arkavo_sbe::PolicyLayer;

/// Lifecycle states for policy candidates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateState {
    /// Newly generated, not yet evaluated
    Pending,
    /// Currently being evaluated alongside production
    Active,
    /// Flagged for promotion consideration (statistically significant)
    FlaggedForPromotion,
    /// Promoted to production policy
    Promoted,
    /// Discarded due to invariant violation or poor performance
    Discarded,
}

impl CandidateState {
    /// Check if the candidate can be evaluated
    pub fn is_evaluable(&self) -> bool {
        matches!(self, Self::Active | Self::FlaggedForPromotion)
    }

    /// Check if the candidate is terminal (no more transitions)
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Promoted | Self::Discarded)
    }
}

/// How the candidate was generated
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GenerationMethod {
    /// Generated via LLM synthesis
    LlmSynthesis {
        /// Model identifier used for generation
        model_id: String,
        /// Hash of the prompt used
        prompt_hash: u64,
    },
    /// Generated via boundary-aware mutation
    BoundaryMutation {
        /// Base candidate that was mutated
        base_id: Uuid,
        /// Description of the mutation applied
        mutation_desc: String,
    },
    /// Generated to remediate a detected anomaly
    AnomalyRemediation {
        /// ID of the anomaly being remediated
        anomaly_id: Uuid,
    },
    /// Manually created candidate
    Manual,
}

impl GenerationMethod {
    /// Get a short description of the generation method
    pub fn description(&self) -> String {
        match self {
            Self::LlmSynthesis { model_id, .. } => format!("LLM: {}", model_id),
            Self::BoundaryMutation { mutation_desc, .. } => {
                format!("Boundary mutation: {}", mutation_desc)
            }
            Self::AnomalyRemediation { anomaly_id } => format!("Anomaly fix: {}", anomaly_id),
            Self::Manual => "Manual".to_string(),
        }
    }
}

/// A candidate policy variant being evaluated counterfactually
#[derive(Debug, Clone)]
pub struct PolicyCandidate {
    /// Unique identifier
    pub id: Uuid,
    /// The policy graph
    pub policy: PolicyLayer,
    /// Current lifecycle state
    pub state: CandidateState,
    /// How this candidate was generated
    pub generation_method: GenerationMethod,
    /// When the candidate was created
    pub created_at: DateTime<Utc>,
    /// When first flagged for promotion (if ever)
    pub first_flagged_at: Option<DateTime<Utc>>,
    /// Number of evaluations performed
    pub evaluation_count: u64,
    /// Number of invariant violations detected
    pub invariant_violations: u32,
}

impl PolicyCandidate {
    /// Create a new candidate in Pending state
    pub fn new(policy: PolicyLayer, generation_method: GenerationMethod) -> Self {
        Self {
            id: Uuid::new_v4(),
            policy,
            state: CandidateState::Pending,
            generation_method,
            created_at: Utc::now(),
            first_flagged_at: None,
            evaluation_count: 0,
            invariant_violations: 0,
        }
    }

    /// Create a candidate with a specific ID (for deserialization)
    pub fn with_id(id: Uuid, policy: PolicyLayer, generation_method: GenerationMethod) -> Self {
        Self {
            id,
            policy,
            state: CandidateState::Pending,
            generation_method,
            created_at: Utc::now(),
            first_flagged_at: None,
            evaluation_count: 0,
            invariant_violations: 0,
        }
    }

    /// Activate the candidate for evaluation
    pub fn activate(&mut self) {
        if self.state == CandidateState::Pending {
            self.state = CandidateState::Active;
        }
    }

    /// Flag the candidate for promotion
    pub fn flag_for_promotion(&mut self) {
        if self.state == CandidateState::Active {
            self.state = CandidateState::FlaggedForPromotion;
            self.first_flagged_at = Some(Utc::now());
        }
    }

    /// Promote the candidate
    pub fn promote(&mut self) {
        if matches!(
            self.state,
            CandidateState::Active | CandidateState::FlaggedForPromotion
        ) {
            self.state = CandidateState::Promoted;
        }
    }

    /// Discard the candidate
    pub fn discard(&mut self) {
        if !self.state.is_terminal() {
            self.state = CandidateState::Discarded;
        }
    }

    /// Record an evaluation
    pub fn record_evaluation(&mut self) {
        self.evaluation_count += 1;
    }

    /// Record an invariant violation
    pub fn record_violation(&mut self) {
        self.invariant_violations += 1;
    }

    /// Get hours since first flagged for promotion
    pub fn hours_since_flagged(&self) -> f64 {
        self.first_flagged_at
            .map(|t| (Utc::now() - t).num_seconds() as f64 / 3600.0)
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_candidate_lifecycle() {
        let policy = PolicyLayer::new(create_simple_graph());
        let mut candidate = PolicyCandidate::new(policy, GenerationMethod::Manual);

        assert_eq!(candidate.state, CandidateState::Pending);
        assert!(!candidate.state.is_evaluable());

        candidate.activate();
        assert_eq!(candidate.state, CandidateState::Active);
        assert!(candidate.state.is_evaluable());

        candidate.flag_for_promotion();
        assert_eq!(candidate.state, CandidateState::FlaggedForPromotion);
        assert!(candidate.first_flagged_at.is_some());

        candidate.promote();
        assert_eq!(candidate.state, CandidateState::Promoted);
        assert!(candidate.state.is_terminal());
    }

    #[test]
    fn test_candidate_discard() {
        let policy = PolicyLayer::new(create_simple_graph());
        let mut candidate = PolicyCandidate::new(policy, GenerationMethod::Manual);

        candidate.activate();
        candidate.discard();

        assert_eq!(candidate.state, CandidateState::Discarded);
        assert!(candidate.state.is_terminal());
    }

    #[test]
    fn test_generation_method_description() {
        let llm = GenerationMethod::LlmSynthesis {
            model_id: "gpt-4".to_string(),
            prompt_hash: 12345,
        };
        assert!(llm.description().contains("gpt-4"));

        let manual = GenerationMethod::Manual;
        assert_eq!(manual.description(), "Manual");
    }

    #[test]
    fn test_evaluation_tracking() {
        let policy = PolicyLayer::new(create_simple_graph());
        let mut candidate = PolicyCandidate::new(policy, GenerationMethod::Manual);

        assert_eq!(candidate.evaluation_count, 0);
        candidate.record_evaluation();
        candidate.record_evaluation();
        assert_eq!(candidate.evaluation_count, 2);
    }
}
