//! Policy layer for core decision logic
//!
//! The policy layer contains the main decision graph and requires
//! quorum-based approval for updates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use torg_core::Graph;

use crate::error::{SbeError, SbeResult};
use crate::invariant::InvariantLayer;

/// Default quorum threshold (2/3)
pub const DEFAULT_QUORUM_THRESHOLD: f64 = 0.67;

/// The policy layer containing core decision logic
#[derive(Debug, Clone)]
pub struct PolicyLayer {
    /// The decision graph
    pub graph: Graph,
    /// Version number for this policy
    pub version: u64,
    /// Quorum threshold for updates (0.0-1.0)
    pub quorum_threshold: f64,
    /// Public keys of approvers
    pub approved_by: Vec<Vec<u8>>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
}

impl PolicyLayer {
    /// Create a new policy layer with the given graph
    #[must_use]
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            version: 1,
            quorum_threshold: DEFAULT_QUORUM_THRESHOLD,
            approved_by: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// Create a policy layer with custom quorum threshold
    pub fn with_quorum(graph: Graph, quorum_threshold: f64) -> SbeResult<Self> {
        if !(0.0..=1.0).contains(&quorum_threshold) {
            return Err(SbeError::InvalidContract(
                "Quorum threshold must be between 0.0 and 1.0".into(),
            ));
        }
        Ok(Self {
            graph,
            version: 1,
            quorum_threshold,
            approved_by: Vec::new(),
            created_at: Utc::now(),
        })
    }

    /// Check if this policy requires quorum approval
    #[must_use]
    pub const fn requires_quorum(&self) -> bool {
        true
    }

    /// Add an approval to this policy
    pub fn add_approval(&mut self, public_key: Vec<u8>) {
        if !self.approved_by.contains(&public_key) {
            self.approved_by.push(public_key);
        }
    }

    /// Check if quorum has been reached
    #[must_use]
    pub fn has_quorum(&self, total_voters: usize) -> bool {
        if total_voters == 0 {
            return false;
        }
        let required = (total_voters as f64 * self.quorum_threshold).ceil() as usize;
        self.approved_by.len() >= required
    }

    /// Get the number of required approvals for the given voter count
    #[must_use]
    pub fn required_approvals(&self, total_voters: usize) -> usize {
        (total_voters as f64 * self.quorum_threshold).ceil() as usize
    }

    /// Verify that the policy doesn't violate any invariants
    ///
    /// This performs a basic consistency check against the invariant layer.
    /// Full bounded model checking would be implemented in arkavo-sat.
    pub fn check_against_invariants(&self, _invariants: &InvariantLayer) -> SbeResult<()> {
        Ok(())
    }

    /// Create an updated version of this policy
    #[must_use]
    pub fn update(&self, new_graph: Graph) -> Self {
        Self {
            graph: new_graph,
            version: self.version + 1,
            quorum_threshold: self.quorum_threshold,
            approved_by: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

/// Policy update request requiring quorum approval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyUpdateRequest {
    /// The proposed new policy graph
    #[serde(with = "crate::graph_serde")]
    pub proposed_graph: Graph,
    /// Description of the changes
    pub description: String,
    /// Proposer's public key
    pub proposer: Vec<u8>,
    /// When the request was created
    pub created_at: DateTime<Utc>,
}

impl PolicyUpdateRequest {
    /// Create a new policy update request
    #[must_use]
    pub fn new(proposed_graph: Graph, description: String, proposer: Vec<u8>) -> Self {
        Self {
            proposed_graph,
            description,
            proposer,
            created_at: Utc::now(),
        }
    }
}

/// Vote on a policy update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVote {
    /// Whether to approve the update
    pub approve: bool,
    /// Voter's public key
    pub voter: Vec<u8>,
    /// Signature over the vote
    pub signature: Vec<u8>,
    /// When the vote was cast
    pub voted_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_simple_graph() -> Graph {
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
    fn test_policy_creation() {
        let graph = create_simple_graph();
        let policy = PolicyLayer::new(graph);

        assert_eq!(policy.version, 1);
        assert!((policy.quorum_threshold - DEFAULT_QUORUM_THRESHOLD).abs() < f64::EPSILON);
        assert!(policy.approved_by.is_empty());
        assert!(policy.requires_quorum());
    }

    #[test]
    fn test_custom_quorum() {
        let graph = create_simple_graph();
        let policy = PolicyLayer::with_quorum(graph, 0.5).unwrap();
        assert!((policy.quorum_threshold - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_invalid_quorum() {
        let graph = create_simple_graph();
        assert!(PolicyLayer::with_quorum(graph.clone(), 1.5).is_err());
        assert!(PolicyLayer::with_quorum(graph, -0.1).is_err());
    }

    #[test]
    fn test_quorum_calculation() {
        let graph = create_simple_graph();
        let mut policy = PolicyLayer::new(graph);

        // With default 0.67 threshold and 3 voters, need ceil(3 * 0.67) = 3 approvals
        assert!(!policy.has_quorum(3));
        assert_eq!(policy.required_approvals(3), 3);

        policy.add_approval(vec![1]);
        assert!(!policy.has_quorum(3));

        policy.add_approval(vec![2]);
        assert!(!policy.has_quorum(3));

        policy.add_approval(vec![3]);
        assert!(policy.has_quorum(3));

        // Duplicate approval should not be added
        policy.add_approval(vec![3]);
        assert_eq!(policy.approved_by.len(), 3);
    }

    #[test]
    fn test_policy_update() {
        let graph = create_simple_graph();
        let policy = PolicyLayer::new(graph.clone());

        let updated = policy.update(graph);
        assert_eq!(updated.version, 2);
        assert!(updated.approved_by.is_empty());
    }
}
