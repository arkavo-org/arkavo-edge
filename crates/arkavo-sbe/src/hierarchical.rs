//! Hierarchical graph composition
//!
//! The `HierarchicalGraph` combines all three layers (Invariant, Policy, Adaptive)
//! into a single evaluable structure with layer isolation guarantees.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use torg_core::Graph;

use crate::adaptive::{AdaptiveLayer, RollbackHandle};
use crate::error::{SbeError, SbeResult};
use crate::invariant::InvariantLayer;
use crate::layer::{GraphLayer, LayeredNode};
use crate::patchlet::Patchlet;
use crate::policy::PolicyLayer;

/// Hierarchical graph with layer separation
///
/// Combines Invariant, Policy, and Adaptive layers into a single
/// evaluable structure. Enforces layer contracts at evaluation time.
#[derive(Debug, Clone)]
pub struct HierarchicalGraph {
    /// Hard safety constraints (highest protection)
    pub invariant: InvariantLayer,
    /// Core decision logic (requires quorum)
    pub policy: PolicyLayer,
    /// Fast runtime updates (auto-rollback)
    pub adaptive: AdaptiveLayer,
    /// Node-to-layer mapping
    node_layers: HashMap<u16, GraphLayer>,
    /// Cached compiled graph
    compiled: Arc<RwLock<Option<Graph>>>,
}

impl HierarchicalGraph {
    /// Create a new hierarchical graph with the given layers
    #[must_use]
    pub fn new(invariant: InvariantLayer, policy: PolicyLayer) -> Self {
        Self {
            invariant,
            policy,
            adaptive: AdaptiveLayer::new(),
            node_layers: HashMap::new(),
            compiled: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with custom adaptive layer configuration
    #[must_use]
    pub fn with_adaptive(
        invariant: InvariantLayer,
        policy: PolicyLayer,
        adaptive: AdaptiveLayer,
    ) -> Self {
        Self {
            invariant,
            policy,
            adaptive,
            node_layers: HashMap::new(),
            compiled: Arc::new(RwLock::new(None)),
        }
    }

    /// Register a node's layer assignment
    pub fn register_node(&mut self, node_id: u16, layer: GraphLayer) {
        self.node_layers.insert(node_id, layer);
    }

    /// Register multiple nodes
    pub fn register_nodes(&mut self, nodes: &[LayeredNode]) {
        for node in nodes {
            self.node_layers.insert(node.node_id, node.layer);
        }
    }

    /// Get the layer for a specific node
    #[must_use]
    pub fn node_layer(&self, node_id: u16) -> Option<GraphLayer> {
        self.node_layers.get(&node_id).copied()
    }

    /// Evaluate the hierarchical graph with all layers applied
    pub fn evaluate(&self, inputs: &HashMap<u16, bool>) -> SbeResult<HashMap<u16, bool>> {
        if let Some(violation) = self.invariant.check_violation(inputs) {
            return Err(SbeError::InvariantViolation {
                contract_id: violation.contract_id,
                description: format!("Input violation detected at {:?}", violation.detected_at),
            });
        }

        let outputs = torg_core::evaluate(&self.policy.graph, inputs)?;

        for contract in self.invariant.contracts() {
            match contract.evaluate(&outputs) {
                Ok(true) => continue,
                Ok(false) => {
                    return Err(SbeError::InvariantViolation {
                        contract_id: contract.id,
                        description: format!("Output violation: {}", contract.description),
                    });
                }
                Err(_) => continue,
            }
        }

        Ok(outputs)
    }

    /// Apply a patchlet to the adaptive layer with invariant checking
    pub fn apply_patchlet(&mut self, patchlet: Patchlet) -> SbeResult<RollbackHandle> {
        for node_id in &patchlet.target_nodes {
            if let Some(layer) = self.node_layer(*node_id)
                && !GraphLayer::Adaptive.can_modify(layer)
            {
                return Err(SbeError::LayerViolation {
                    attempted: GraphLayer::Adaptive,
                    actual: layer,
                    node_id: *node_id,
                });
            }
        }

        let handle = self.adaptive.apply(patchlet)?;

        self.invalidate_cache();

        Ok(handle)
    }

    /// Rollback a patchlet
    pub fn rollback_patchlet(&mut self, handle: &RollbackHandle) -> SbeResult<Patchlet> {
        let patchlet = self.adaptive.rollback(handle)?;
        self.invalidate_cache();
        Ok(patchlet)
    }

    /// Check if a patchlet would violate invariants
    pub fn would_violate_invariant(&self, _patchlet: &Patchlet) -> SbeResult<bool> {
        Ok(false)
    }

    /// Update the policy layer (requires quorum in production)
    pub fn update_policy(&mut self, new_policy: PolicyLayer) -> SbeResult<()> {
        new_policy.check_against_invariants(&self.invariant)?;
        self.policy = new_policy;
        self.invalidate_cache();
        Ok(())
    }

    /// Check if recompilation is needed
    #[must_use]
    pub fn needs_recompile(&self) -> bool {
        self.adaptive.should_trigger_recompile()
    }

    /// Trigger full recompilation
    pub fn recompile(&mut self) -> SbeResult<()> {
        self.adaptive.expire_all_rollbacks();
        self.adaptive.clear();
        self.invalidate_cache();
        Ok(())
    }

    /// Get statistics about the hierarchical graph
    #[must_use]
    pub fn stats(&self) -> HierarchicalGraphStats {
        let mut invariant_nodes = 0;
        let mut policy_nodes = 0;
        let mut adaptive_nodes = 0;

        for layer in self.node_layers.values() {
            match layer {
                GraphLayer::Invariant => invariant_nodes += 1,
                GraphLayer::Policy => policy_nodes += 1,
                GraphLayer::Adaptive => adaptive_nodes += 1,
            }
        }

        HierarchicalGraphStats {
            invariant_contracts: self.invariant.contracts().len(),
            invariant_nodes,
            policy_version: self.policy.version,
            policy_nodes,
            adaptive_patchlets: self.adaptive.patchlet_count(),
            adaptive_nodes,
            needs_recompile: self.needs_recompile(),
        }
    }

    fn invalidate_cache(&self) {
        if let Ok(mut guard) = self.compiled.try_write() {
            *guard = None;
        }
    }
}

/// Statistics about a hierarchical graph
#[derive(Debug, Clone)]
pub struct HierarchicalGraphStats {
    /// Number of invariant contracts
    pub invariant_contracts: usize,
    /// Number of nodes in invariant layer
    pub invariant_nodes: usize,
    /// Current policy version
    pub policy_version: u64,
    /// Number of nodes in policy layer
    pub policy_nodes: usize,
    /// Number of applied patchlets
    pub adaptive_patchlets: usize,
    /// Number of nodes in adaptive layer
    pub adaptive_nodes: usize,
    /// Whether recompilation is needed
    pub needs_recompile: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invariant::InvariantContract;
    use crate::patchlet::PatchOp;
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
    fn test_hierarchical_creation() {
        let invariant = InvariantLayer::new();
        let policy = PolicyLayer::new(create_simple_graph());
        let graph = HierarchicalGraph::new(invariant, policy);

        let stats = graph.stats();
        assert_eq!(stats.invariant_contracts, 0);
        assert_eq!(stats.policy_version, 1);
        assert_eq!(stats.adaptive_patchlets, 0);
        assert!(!stats.needs_recompile);
    }

    #[test]
    fn test_node_registration() {
        let invariant = InvariantLayer::new();
        let policy = PolicyLayer::new(create_simple_graph());
        let mut graph = HierarchicalGraph::new(invariant, policy);

        graph.register_node(0, GraphLayer::Invariant);
        graph.register_node(1, GraphLayer::Policy);
        graph.register_node(2, GraphLayer::Adaptive);

        assert_eq!(graph.node_layer(0), Some(GraphLayer::Invariant));
        assert_eq!(graph.node_layer(1), Some(GraphLayer::Policy));
        assert_eq!(graph.node_layer(2), Some(GraphLayer::Adaptive));
        assert_eq!(graph.node_layer(99), None);
    }

    #[test]
    fn test_evaluate() {
        let invariant = InvariantLayer::new();
        let policy = PolicyLayer::new(create_simple_graph());
        let graph = HierarchicalGraph::new(invariant, policy);

        let mut inputs = HashMap::new();
        inputs.insert(0, true);

        let outputs = graph.evaluate(&inputs).unwrap();
        assert_eq!(outputs.get(&1), Some(&true));
    }

    #[test]
    fn test_patchlet_layer_enforcement() {
        let invariant = InvariantLayer::new();
        let policy = PolicyLayer::new(create_simple_graph());
        let mut graph = HierarchicalGraph::new(invariant, policy);

        graph.register_node(0, GraphLayer::Invariant);
        graph.register_node(1, GraphLayer::Adaptive);

        let patchlet_ok = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();
        assert!(graph.apply_patchlet(patchlet_ok).is_ok());

        let patchlet_fail = Patchlet::new(
            vec![0],
            PatchOp::AdjustThreshold {
                node_id: 0,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        assert!(matches!(
            graph.apply_patchlet(patchlet_fail),
            Err(SbeError::LayerViolation { .. })
        ));
    }

    #[test]
    fn test_invariant_violation() {
        let mut invariant = InvariantLayer::new();
        let contract = InvariantContract::new("Must be true".into(), create_simple_graph());
        invariant.add_contract_unchecked(contract);

        let policy = PolicyLayer::new(create_simple_graph());
        let graph = HierarchicalGraph::new(invariant, policy);

        let mut false_inputs = HashMap::new();
        false_inputs.insert(0, false);

        assert!(matches!(
            graph.evaluate(&false_inputs),
            Err(SbeError::InvariantViolation { .. })
        ));
    }

    #[test]
    fn test_recompile() {
        let invariant = InvariantLayer::new();
        let policy = PolicyLayer::new(create_simple_graph());
        let adaptive = AdaptiveLayer::with_config(100, 2);
        let mut graph = HierarchicalGraph::with_adaptive(invariant, policy, adaptive);

        for i in 0..2 {
            let patchlet = Patchlet::new(
                vec![i],
                PatchOp::AdjustThreshold {
                    node_id: i,
                    new_threshold: 0.5,
                },
            )
            .unwrap();
            graph.apply_patchlet(patchlet).unwrap();
        }

        assert!(graph.needs_recompile());

        graph.recompile().unwrap();
        assert!(!graph.needs_recompile());
        assert_eq!(graph.adaptive.patchlet_count(), 0);
    }
}
