//! Patchlet types for minimal graph modifications
//!
//! Patchlets are small, scoped modifications that can be applied
//! to the adaptive layer without full recompilation.

use arkavo_crypto::AgentKeypair;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use torg_core::Graph;
use uuid::Uuid;

use crate::error::{SbeError, SbeResult};

/// Default maximum number of nodes a patchlet can affect
pub const DEFAULT_MAX_PATCHLET_NODES: usize = 10;

/// Default maximum number of patchlets before recompilation
pub const DEFAULT_MAX_PATCHLETS: usize = 20;

/// Minimal graph modification for fast updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patchlet {
    /// Unique identifier
    pub id: Uuid,
    /// Target node IDs to modify
    pub target_nodes: Vec<u16>,
    /// Type of modification
    pub modification: PatchOp,
    /// Cryptographic signature (optional for adaptive layer)
    pub signature: Option<Vec<u8>>,
    /// Signer public key if signed
    pub signer_public_key: Option<Vec<u8>>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
}

/// Patch operation types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PatchOp {
    /// Insert a guard node before target nodes
    InsertGuard {
        /// The guard predicate graph
        #[serde(with = "crate::graph_serde")]
        guard_predicate: Graph,
    },

    /// Adjust a threshold value on a target node
    AdjustThreshold {
        /// The node to adjust
        node_id: u16,
        /// New threshold value (0.0-1.0)
        new_threshold: f64,
    },

    /// Add a denylist predicate
    AddDenylist {
        /// The denylist predicate graph
        #[serde(with = "crate::graph_serde")]
        predicate: Graph,
    },

    /// Replace a node with an equivalent subgraph
    ReplaceNode {
        /// The node to replace
        old_id: u16,
        /// The replacement subgraph
        #[serde(with = "crate::graph_serde")]
        new_graph: Graph,
    },
}

impl Patchlet {
    /// Create a new patchlet
    pub fn new(target_nodes: Vec<u16>, modification: PatchOp) -> SbeResult<Self> {
        let patchlet = Self {
            id: Uuid::new_v4(),
            target_nodes,
            modification,
            signature: None,
            signer_public_key: None,
            created_at: Utc::now(),
        };
        patchlet.validate_scope(DEFAULT_MAX_PATCHLET_NODES)?;
        Ok(patchlet)
    }

    /// Create a patchlet with custom scope limit
    pub fn with_scope_limit(
        target_nodes: Vec<u16>,
        modification: PatchOp,
        max_nodes: usize,
    ) -> SbeResult<Self> {
        let patchlet = Self {
            id: Uuid::new_v4(),
            target_nodes,
            modification,
            signature: None,
            signer_public_key: None,
            created_at: Utc::now(),
        };
        patchlet.validate_scope(max_nodes)?;
        Ok(patchlet)
    }

    /// Validate that the patchlet doesn't exceed scope limits
    pub fn validate_scope(&self, max_nodes: usize) -> SbeResult<()> {
        let affected = self.affected_node_count();
        if affected > max_nodes {
            return Err(SbeError::ScopeExceeded {
                affected,
                max: max_nodes,
            });
        }
        Ok(())
    }

    /// Get the number of nodes this patchlet affects
    #[must_use]
    pub fn affected_node_count(&self) -> usize {
        self.target_nodes.len()
    }

    /// Sign this patchlet
    pub fn sign(&mut self, keypair: &AgentKeypair) {
        let content = self.content_to_sign();
        self.signature = Some(keypair.sign(&content));
        self.signer_public_key = Some(keypair.public_key().to_bytes());
    }

    /// Check if this patchlet is signed
    #[must_use]
    pub fn is_signed(&self) -> bool {
        self.signature.is_some() && self.signer_public_key.is_some()
    }

    /// Serialize this patchlet to bytes
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize a patchlet from bytes
    pub fn from_bytes(bytes: &[u8]) -> SbeResult<Self> {
        serde_json::from_slice(bytes).map_err(|e| SbeError::Serialization(e.to_string()))
    }

    fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.id.as_bytes());
        for node_id in &self.target_nodes {
            content.extend(node_id.to_le_bytes());
        }
        content.extend(self.created_at.timestamp().to_le_bytes());
        content
    }
}

/// A patchlet that has been applied to a graph
#[derive(Debug, Clone)]
pub struct AppliedPatchlet {
    /// The patchlet that was applied
    pub patchlet: Patchlet,
    /// When the patchlet was applied
    pub applied_at: DateTime<Utc>,
    /// Whether rollback is still available
    pub rollback_available: bool,
}

impl AppliedPatchlet {
    /// Create a new applied patchlet record
    #[must_use]
    pub fn new(patchlet: Patchlet) -> Self {
        Self {
            patchlet,
            applied_at: Utc::now(),
            rollback_available: true,
        }
    }

    /// Mark this patchlet as no longer rollbackable
    pub fn expire_rollback(&mut self) {
        self.rollback_available = false;
    }
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
    fn test_patchlet_creation() {
        let patchlet = Patchlet::new(
            vec![1, 2, 3],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        );
        assert!(patchlet.is_ok());
        let p = patchlet.unwrap();
        assert_eq!(p.target_nodes.len(), 3);
        assert!(!p.is_signed());
    }

    #[test]
    fn test_patchlet_scope_exceeded() {
        let target_nodes: Vec<u16> = (0..15).collect();
        let patchlet = Patchlet::new(
            target_nodes,
            PatchOp::AdjustThreshold {
                node_id: 0,
                new_threshold: 0.5,
            },
        );
        assert!(matches!(patchlet, Err(SbeError::ScopeExceeded { .. })));
    }

    #[test]
    fn test_patchlet_signing() {
        let mut patchlet = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        let keypair = AgentKeypair::generate();
        patchlet.sign(&keypair);

        assert!(patchlet.is_signed());
        assert!(patchlet.signature.is_some());
        assert!(patchlet.signer_public_key.is_some());
    }

    #[test]
    fn test_patchlet_serialization() {
        let patchlet = Patchlet::new(
            vec![1, 2],
            PatchOp::InsertGuard {
                guard_predicate: create_simple_graph(),
            },
        )
        .unwrap();

        let bytes = patchlet.to_bytes();
        assert!(!bytes.is_empty());

        let restored = Patchlet::from_bytes(&bytes).unwrap();
        assert_eq!(restored.id, patchlet.id);
        assert_eq!(restored.target_nodes, patchlet.target_nodes);
    }

    #[test]
    fn test_applied_patchlet() {
        let patchlet = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        let mut applied = AppliedPatchlet::new(patchlet);
        assert!(applied.rollback_available);

        applied.expire_rollback();
        assert!(!applied.rollback_available);
    }
}
