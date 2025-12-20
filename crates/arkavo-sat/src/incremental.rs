//! Incremental verification for modified graphs
//!
//! Tracks graph modifications and determines which outputs need re-probing,
//! avoiding full re-verification when only part of the graph changes.

use std::collections::{HashMap, HashSet};

use torg_core::Graph;

/// Result of incremental analysis
#[derive(Debug, Clone)]
pub struct IncrementalAnalysis {
    /// Output IDs that need re-probing
    pub affected_outputs: Vec<u16>,
    /// Estimated work reduction as a percentage (0.0 to 1.0)
    pub savings_estimate: f64,
}

/// Tracks modifications to a graph for incremental verification
pub struct ModificationTracker {
    /// Nodes that have been modified since last verification
    modified_nodes: HashSet<u16>,
    /// Mapping from node to its dependents (reverse dependency graph)
    dependents: HashMap<u16, HashSet<u16>>,
    /// Current graph version (incremented on any modification)
    current_version: u64,
    /// Last verified version for each output
    verified_versions: HashMap<u16, u64>,
    /// Total node count (for savings calculation)
    total_nodes: usize,
}

impl ModificationTracker {
    /// Create a new tracker
    pub fn new() -> Self {
        Self {
            modified_nodes: HashSet::new(),
            dependents: HashMap::new(),
            current_version: 0,
            verified_versions: HashMap::new(),
            total_nodes: 0,
        }
    }

    /// Build dependency graph from a TØRG graph
    ///
    /// Creates reverse edges: for each node, tracks which nodes depend on it.
    pub fn build_dependencies(&mut self, graph: &Graph) {
        self.dependents.clear();
        self.total_nodes = graph.nodes.len() + graph.inputs.len();

        // Initialize dependents for all nodes
        for &input_id in &graph.inputs {
            self.dependents.entry(input_id).or_default();
        }

        for node in &graph.nodes {
            self.dependents.entry(node.id).or_default();

            // Add reverse edges for sources
            if let torg_core::token::Source::Id(src_id) = node.left {
                self.dependents.entry(src_id).or_default().insert(node.id);
            }
            if let torg_core::token::Source::Id(src_id) = node.right {
                self.dependents.entry(src_id).or_default().insert(node.id);
            }
        }
    }

    /// Mark a node as modified
    pub fn mark_modified(&mut self, node_id: u16) {
        self.modified_nodes.insert(node_id);
        self.current_version += 1;
    }

    /// Mark a node as added with its dependencies
    pub fn mark_added(&mut self, node_id: u16, left_src: Option<u16>, right_src: Option<u16>) {
        self.modified_nodes.insert(node_id);
        self.dependents.entry(node_id).or_default();

        if let Some(left) = left_src {
            self.dependents.entry(left).or_default().insert(node_id);
        }
        if let Some(right) = right_src {
            self.dependents.entry(right).or_default().insert(node_id);
        }

        self.current_version += 1;
        self.total_nodes += 1;
    }

    /// Mark a node as removed
    pub fn mark_removed(&mut self, node_id: u16) {
        self.modified_nodes.insert(node_id);

        // Remove from dependents
        self.dependents.remove(&node_id);

        // Remove as dependent from others
        for deps in self.dependents.values_mut() {
            deps.remove(&node_id);
        }

        self.current_version += 1;
        if self.total_nodes > 0 {
            self.total_nodes -= 1;
        }
    }

    /// Get outputs affected by modifications since last verification
    pub fn get_affected_outputs(&self, graph: &Graph) -> IncrementalAnalysis {
        if self.modified_nodes.is_empty() {
            return IncrementalAnalysis {
                affected_outputs: vec![],
                savings_estimate: 1.0,
            };
        }

        // Propagate modifications through dependency graph
        let all_affected = self.propagate_modifications();

        // Filter to just outputs
        let output_set: HashSet<u16> = graph.outputs.iter().copied().collect();
        let affected_outputs: Vec<u16> = all_affected.intersection(&output_set).copied().collect();

        // Calculate savings
        let total_outputs = graph.outputs.len();
        let savings = if total_outputs > 0 {
            1.0 - (affected_outputs.len() as f64 / total_outputs as f64)
        } else {
            0.0
        };

        IncrementalAnalysis {
            affected_outputs,
            savings_estimate: savings,
        }
    }

    /// Propagate modifications through the dependency graph
    fn propagate_modifications(&self) -> HashSet<u16> {
        let mut affected = self.modified_nodes.clone();
        let mut frontier: Vec<u16> = self.modified_nodes.iter().copied().collect();

        while let Some(node) = frontier.pop() {
            if let Some(deps) = self.dependents.get(&node) {
                for &dep in deps {
                    if affected.insert(dep) {
                        frontier.push(dep);
                    }
                }
            }
        }

        affected
    }

    /// Mark outputs as verified at current version
    pub fn mark_verified(&mut self, output_ids: &[u16]) {
        for &output_id in output_ids {
            self.verified_versions
                .insert(output_id, self.current_version);
        }
        self.modified_nodes.clear();
    }

    /// Check if an output needs re-verification
    pub fn needs_verification(&self, output_id: u16) -> bool {
        match self.verified_versions.get(&output_id) {
            Some(&ver) => ver < self.current_version,
            None => true, // Never verified
        }
    }

    /// Reset tracker (e.g., for full re-verification)
    pub fn reset(&mut self) {
        self.modified_nodes.clear();
        self.verified_versions.clear();
        self.current_version = 0;
    }

    /// Get current version
    pub fn version(&self) -> u64 {
        self.current_version
    }

    /// Get number of modified nodes
    pub fn modified_count(&self) -> usize {
        self.modified_nodes.len()
    }
}

impl Default for ModificationTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_chain_graph() -> Graph {
        // Linear chain: input -> node1 -> node2 -> output
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();

        // Node 1: OR(0, 0)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        // Node 2: OR(1, 1)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(2)).unwrap();

        builder.finish().unwrap()
    }

    fn create_diamond_graph() -> Graph {
        // Diamond: input -> (node1, node2) -> node3 (output)
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();

        // Node 1: OR(0, 0)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        // Node 2: NOR(0, 0)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        // Node 3: OR(1, 2)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(3)).unwrap();

        builder.finish().unwrap()
    }

    #[test]
    fn test_build_dependencies() {
        let graph = create_chain_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        // Node 0 (input) should have node 1 as dependent
        assert!(tracker.dependents.get(&0).unwrap().contains(&1));

        // Node 1 should have node 2 as dependent
        assert!(tracker.dependents.get(&1).unwrap().contains(&2));
    }

    #[test]
    fn test_propagate_modifications() {
        let graph = create_chain_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        // Modify input
        tracker.mark_modified(0);

        let analysis = tracker.get_affected_outputs(&graph);

        // Should affect the output
        assert!(analysis.affected_outputs.contains(&2));
    }

    #[test]
    fn test_diamond_propagation() {
        let graph = create_diamond_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        // Modify node 1
        tracker.mark_modified(1);

        let analysis = tracker.get_affected_outputs(&graph);

        // Should affect output (node 3) through node 1
        assert!(analysis.affected_outputs.contains(&3));
    }

    #[test]
    fn test_no_modifications() {
        let graph = create_chain_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        let analysis = tracker.get_affected_outputs(&graph);

        // No modifications -> no affected outputs
        assert!(analysis.affected_outputs.is_empty());
        assert!((analysis.savings_estimate - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_mark_verified() {
        let graph = create_chain_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        tracker.mark_modified(0);
        assert!(tracker.needs_verification(2));

        tracker.mark_verified(&[2]);
        assert!(!tracker.needs_verification(2));
        assert_eq!(tracker.modified_count(), 0);
    }

    #[test]
    fn test_version_tracking() {
        let mut tracker = ModificationTracker::new();

        assert_eq!(tracker.version(), 0);

        tracker.mark_modified(1);
        assert_eq!(tracker.version(), 1);

        tracker.mark_modified(2);
        assert_eq!(tracker.version(), 2);
    }

    #[test]
    fn test_mark_added() {
        let graph = create_chain_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        let initial_version = tracker.version();

        // Add a new node that depends on node 2
        tracker.mark_added(10, Some(2), None);

        assert!(tracker.version() > initial_version);

        // New node should be in modified set
        assert!(tracker.modified_nodes.contains(&10));

        // Node 2 should have new node as dependent
        assert!(tracker.dependents.get(&2).unwrap().contains(&10));
    }

    #[test]
    fn test_mark_removed() {
        let graph = create_chain_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        // Remove node 1
        tracker.mark_removed(1);

        // Node 1 should not be in dependents
        assert!(!tracker.dependents.contains_key(&1));

        // Node 0 should not have node 1 as dependent
        assert!(!tracker.dependents.get(&0).unwrap().contains(&1));
    }

    #[test]
    fn test_reset() {
        let mut tracker = ModificationTracker::new();
        tracker.mark_modified(1);
        tracker.verified_versions.insert(1, 1);

        tracker.reset();

        assert_eq!(tracker.version(), 0);
        assert!(tracker.modified_nodes.is_empty());
        assert!(tracker.verified_versions.is_empty());
    }

    #[test]
    fn test_savings_estimate() {
        let graph = create_diamond_graph();
        let mut tracker = ModificationTracker::new();
        tracker.build_dependencies(&graph);

        // No modifications = 100% savings
        let analysis = tracker.get_affected_outputs(&graph);
        assert!((analysis.savings_estimate - 1.0).abs() < 0.001);

        // Modify something that affects output
        tracker.mark_modified(0);
        let analysis = tracker.get_affected_outputs(&graph);
        // 1 output affected out of 1 total = 0% savings
        assert!((analysis.savings_estimate - 0.0).abs() < 0.001);
    }
}
