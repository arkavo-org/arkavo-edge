//! Graph layer types for hierarchical policy structure
//!
//! SBE defines three hierarchical layers:
//! - Invariant: Hard safety constraints (human-only updates)
//! - Policy: Core decision logic (quorum-based updates)
//! - Adaptive: Fast threshold/weight updates (auto-rollback)

use serde::{Deserialize, Serialize};
use std::fmt;

/// Layer classification for hierarchical graph structure
///
/// Layers are ordered by immutability: Invariant > Policy > Adaptive
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GraphLayer {
    /// Hard safety constraints that can only be updated by humans
    /// with formal proof verification. These constraints represent
    /// fundamental security invariants that must never be violated.
    Invariant,

    /// Core decision logic that requires quorum-based updates
    /// (default 2/3 approval). Policy changes undergo bounded
    /// model checking before deployment.
    Policy,

    /// Fast threshold and weight updates that support automatic
    /// rollback within a configurable timeout (default 100ms).
    /// Used for runtime adaptation without human intervention.
    #[default]
    Adaptive,
}

impl GraphLayer {
    /// Returns true if this layer can be modified by automated systems
    #[must_use]
    pub const fn is_automatable(&self) -> bool {
        matches!(self, Self::Adaptive)
    }

    /// Returns true if this layer requires quorum consensus for updates
    #[must_use]
    pub const fn requires_quorum(&self) -> bool {
        matches!(self, Self::Policy)
    }

    /// Returns true if this layer requires human approval
    #[must_use]
    pub const fn requires_human_approval(&self) -> bool {
        matches!(self, Self::Invariant)
    }

    /// Returns the protection level (higher = more protected)
    #[must_use]
    pub const fn protection_level(&self) -> u8 {
        match self {
            Self::Invariant => 3,
            Self::Policy => 2,
            Self::Adaptive => 1,
        }
    }

    /// Check if this layer can modify nodes in the target layer
    #[must_use]
    pub const fn can_modify(&self, target: Self) -> bool {
        self.protection_level() >= target.protection_level()
    }
}

impl fmt::Display for GraphLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invariant => write!(f, "Invariant"),
            Self::Policy => write!(f, "Policy"),
            Self::Adaptive => write!(f, "Adaptive"),
        }
    }
}

/// Node with layer annotation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayeredNode {
    /// The node identifier in the graph
    pub node_id: u16,
    /// Which layer this node belongs to
    pub layer: GraphLayer,
    /// Optional reference to an invariant guard node
    pub guard_node: Option<u16>,
}

impl LayeredNode {
    /// Create a new layered node
    #[must_use]
    pub const fn new(node_id: u16, layer: GraphLayer) -> Self {
        Self {
            node_id,
            layer,
            guard_node: None,
        }
    }

    /// Create a new layered node with a guard
    #[must_use]
    pub const fn with_guard(node_id: u16, layer: GraphLayer, guard_node: u16) -> Self {
        Self {
            node_id,
            layer,
            guard_node: Some(guard_node),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_ordering() {
        assert!(GraphLayer::Invariant.protection_level() > GraphLayer::Policy.protection_level());
        assert!(GraphLayer::Policy.protection_level() > GraphLayer::Adaptive.protection_level());
    }

    #[test]
    fn test_layer_can_modify() {
        assert!(GraphLayer::Invariant.can_modify(GraphLayer::Invariant));
        assert!(GraphLayer::Invariant.can_modify(GraphLayer::Policy));
        assert!(GraphLayer::Invariant.can_modify(GraphLayer::Adaptive));

        assert!(!GraphLayer::Policy.can_modify(GraphLayer::Invariant));
        assert!(GraphLayer::Policy.can_modify(GraphLayer::Policy));
        assert!(GraphLayer::Policy.can_modify(GraphLayer::Adaptive));

        assert!(!GraphLayer::Adaptive.can_modify(GraphLayer::Invariant));
        assert!(!GraphLayer::Adaptive.can_modify(GraphLayer::Policy));
        assert!(GraphLayer::Adaptive.can_modify(GraphLayer::Adaptive));
    }

    #[test]
    fn test_layer_properties() {
        assert!(!GraphLayer::Invariant.is_automatable());
        assert!(!GraphLayer::Policy.is_automatable());
        assert!(GraphLayer::Adaptive.is_automatable());

        assert!(!GraphLayer::Invariant.requires_quorum());
        assert!(GraphLayer::Policy.requires_quorum());
        assert!(!GraphLayer::Adaptive.requires_quorum());

        assert!(GraphLayer::Invariant.requires_human_approval());
        assert!(!GraphLayer::Policy.requires_human_approval());
        assert!(!GraphLayer::Adaptive.requires_human_approval());
    }

    #[test]
    fn test_serialization() {
        let layer = GraphLayer::Policy;
        let json = serde_json::to_string(&layer).unwrap();
        assert_eq!(json, "\"policy\"");

        let deserialized: GraphLayer = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, layer);
    }

    #[test]
    fn test_layered_node() {
        let node = LayeredNode::new(5, GraphLayer::Policy);
        assert_eq!(node.node_id, 5);
        assert_eq!(node.layer, GraphLayer::Policy);
        assert!(node.guard_node.is_none());

        let guarded = LayeredNode::with_guard(10, GraphLayer::Adaptive, 5);
        assert_eq!(guarded.guard_node, Some(5));
    }
}
