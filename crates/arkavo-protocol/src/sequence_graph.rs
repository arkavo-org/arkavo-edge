#[derive(Debug, Clone)]
pub struct ActionNode {
    pub tool_name: String,
    pub params_hash: u64,
    pub taint_labels: Vec<String>,
    pub sequence_number: u64,
}

#[derive(Debug, Clone)]
pub struct ActionEdge {
    pub from: u64,
    pub to: u64,
}

/// SEQ-004: Directed action graph built per session
pub struct SequenceGraph {
    nodes: Vec<ActionNode>,
    edges: Vec<ActionEdge>,
}

impl SequenceGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_node(&mut self, _node: ActionNode) -> u64 {
        0
    }

    pub fn add_edge(&mut self, _from: u64, _to: u64) {}

    pub fn nodes(&self) -> &[ActionNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[ActionEdge] {
        &self.edges
    }

    pub fn node_count(&self) -> usize {
        0
    }
}

pub struct SequenceGraphBuilder {
    _session_id: String,
    graph: SequenceGraph,
    _next_sequence: u64,
}

impl SequenceGraphBuilder {
    pub fn new(session_id: &str) -> Self {
        Self {
            _session_id: session_id.to_string(),
            graph: SequenceGraph::new(),
            _next_sequence: 0,
        }
    }

    /// Record a completed tool call as a node in the action graph
    pub fn record_action(
        &mut self,
        _tool_name: &str,
        _params_hash: u64,
        _taint_labels: Vec<String>,
    ) -> u64 {
        0
    }

    /// Connect data flow between two actions
    pub fn connect(&mut self, _from: u64, _to: u64) {}

    /// Get the current session graph
    pub fn graph(&self) -> &SequenceGraph {
        &self.graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SEQ-004: Build directed action graph per session
    // =========================================================================

    #[test]
    fn node_added_on_tool_call() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        let id = builder.record_action("read_file", 123, vec![]);
        assert_eq!(builder.graph().node_count(), 1);
        assert_eq!(id, 0);
    }

    #[test]
    fn edges_connect_data_flow() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        let a = builder.record_action("read_file", 123, vec![]);
        let b = builder.record_action("summarize", 456, vec!["internal".into()]);
        builder.connect(a, b);
        assert_eq!(builder.graph().edges().len(), 1);
    }

    #[test]
    fn node_metadata_includes_taint_labels() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        builder.record_action("read_db", 789, vec!["pii".into(), "internal".into()]);
        let nodes = builder.graph().nodes();
        assert_eq!(nodes[0].taint_labels.len(), 2);
    }

    #[test]
    fn sequence_numbers_monotonically_increase() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        builder.record_action("a", 1, vec![]);
        builder.record_action("b", 2, vec![]);
        builder.record_action("c", 3, vec![]);
        let nodes = builder.graph().nodes();
        assert!(nodes[0].sequence_number < nodes[1].sequence_number);
        assert!(nodes[1].sequence_number < nodes[2].sequence_number);
    }

    #[test]
    fn graph_updated_atomically() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        builder.record_action("read_file", 100, vec!["internal".into()]);
        let count_before = builder.graph().node_count();
        builder.record_action("write_file", 200, vec![]);
        let count_after = builder.graph().node_count();
        assert_eq!(count_after, count_before + 1);
    }
}
