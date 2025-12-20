//! Symbolic Boundary Evolution (SBE) - Hierarchical Policy Layers
//!
//! SBE extends TØRG with hierarchical layer composition for adaptive
//! policy systems with formal verification guarantees.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │              Invariant Layer                     │
//! │  Hard safety constraints (human-only updates)   │
//! │  Ed25519 signed contracts, formal verification  │
//! └───────────────────────┬─────────────────────────┘
//!                         │
//! ┌───────────────────────▼─────────────────────────┐
//! │               Policy Layer                       │
//! │  Core decision logic (quorum-based updates)     │
//! │  2/3 approval required, bounded model checking  │
//! └───────────────────────┬─────────────────────────┘
//!                         │
//! ┌───────────────────────▼─────────────────────────┐
//! │              Adaptive Layer                      │
//! │  Fast threshold/weight updates (auto-rollback)  │
//! │  100ms timeout, max 20 patchlets before recompile│
//! └─────────────────────────────────────────────────┘
//! ```
//!
//! # Key Features
//!
//! - **Layer Isolation**: Strict type-safe separation between layers
//! - **Contract Enforcement**: Patchlets cannot modify higher layers
//! - **Auto-Rollback**: Adaptive changes can be rolled back within timeout
//! - **Invariant Checking**: Safety constraints verified at evaluation time
//!
//! # Example
//!
//! ```ignore
//! use arkavo_sbe::{HierarchicalGraph, InvariantLayer, PolicyLayer, Patchlet, PatchOp};
//! use arkavo_sbe::layer::GraphLayer;
//!
//! // Create layers
//! let invariant = InvariantLayer::new();
//! let policy = PolicyLayer::new(my_decision_graph);
//!
//! // Build hierarchical graph
//! let mut graph = HierarchicalGraph::new(invariant, policy);
//!
//! // Register node layers
//! graph.register_node(0, GraphLayer::Policy);
//! graph.register_node(1, GraphLayer::Adaptive);
//!
//! // Apply adaptive patchlet
//! let patchlet = Patchlet::new(
//!     vec![1],
//!     PatchOp::AdjustThreshold { node_id: 1, new_threshold: 0.7 },
//! )?;
//! let handle = graph.apply_patchlet(patchlet)?;
//!
//! // Evaluate with inputs
//! let outputs = graph.evaluate(&inputs)?;
//!
//! // Rollback if needed (within timeout)
//! graph.rollback_patchlet(&handle)?;
//! ```

mod adaptive;
mod error;
pub mod graph_serde;
mod hierarchical;
pub mod invariant;
pub mod layer;
mod patchlet;
mod policy;

// Re-export main types
pub use adaptive::{AdaptiveLayer, DEFAULT_ROLLBACK_TIMEOUT_MS, RollbackHandle};
pub use error::{SbeError, SbeResult};
pub use hierarchical::{HierarchicalGraph, HierarchicalGraphStats};
pub use invariant::{InvariantContract, InvariantLayer, ViolationAttempt, ViolationSource};
pub use layer::{GraphLayer, LayeredNode};
pub use patchlet::{
    AppliedPatchlet, DEFAULT_MAX_PATCHLET_NODES, DEFAULT_MAX_PATCHLETS, PatchOp, Patchlet,
};
pub use policy::{DEFAULT_QUORUM_THRESHOLD, PolicyLayer, PolicyUpdateRequest, PolicyVote};

// Re-export torg-core types for convenience
pub use torg_core::{BuildError, Builder, EvalError, Graph, Token};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    fn test_full_workflow() {
        let invariant = InvariantLayer::new();
        let policy = PolicyLayer::new(create_simple_graph());
        let mut graph = HierarchicalGraph::new(invariant, policy);

        graph.register_node(0, GraphLayer::Policy);
        graph.register_node(1, GraphLayer::Adaptive);

        let mut inputs = HashMap::new();
        inputs.insert(0, true);
        let outputs = graph.evaluate(&inputs).unwrap();
        assert_eq!(outputs.get(&1), Some(&true));

        let patchlet = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        let handle = graph.apply_patchlet(patchlet).unwrap();
        assert!(!handle.is_expired());

        let stats = graph.stats();
        assert_eq!(stats.adaptive_patchlets, 1);
    }
}
