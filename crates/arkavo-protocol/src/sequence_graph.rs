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

#[derive(Default)]
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

    pub fn add_node(&mut self, node: ActionNode) -> u64 {
        let id = self.nodes.len() as u64;
        self.nodes.push(node);
        id
    }

    pub fn add_edge(&mut self, from: u64, to: u64) {
        self.edges.push(ActionEdge { from, to });
    }

    pub fn nodes(&self) -> &[ActionNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[ActionEdge] {
        &self.edges
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

pub struct SequenceGraphBuilder {
    _session_id: String,
    graph: SequenceGraph,
    next_sequence: u64,
}

impl SequenceGraphBuilder {
    pub fn new(session_id: &str) -> Self {
        Self {
            _session_id: session_id.to_string(),
            graph: SequenceGraph::new(),
            next_sequence: 0,
        }
    }

    pub fn record_action(
        &mut self,
        tool_name: &str,
        params_hash: u64,
        taint_labels: Vec<String>,
    ) -> u64 {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        let node = ActionNode {
            tool_name: tool_name.to_string(),
            params_hash,
            taint_labels,
            sequence_number: seq,
        };
        self.graph.add_node(node)
    }

    pub fn connect(&mut self, from: u64, to: u64) {
        self.graph.add_edge(from, to);
    }

    pub fn graph(&self) -> &SequenceGraph {
        &self.graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SEQ-004")]
    #[test]
    fn graph_node_count_increments_after_recording_action() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        builder.record_action("read_file", 123, vec![]);
        assert_eq!(builder.graph().node_count(), 1);
    }

    #[spec("SEQ-004")]
    #[test]
    fn edges_connect_data_flow_between_actions() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        let a = builder.record_action("read_file", 123, vec![]);
        let b = builder.record_action("summarize", 456, vec!["internal".into()]);
        builder.connect(a, b);
        assert_eq!(builder.graph().edges().len(), 1);
    }

    #[spec("SEQ-004")]
    #[test]
    fn recorded_node_preserves_taint_labels() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        builder.record_action("read_db", 789, vec!["pii".into(), "internal".into()]);
        let nodes = builder.graph().nodes();
        assert_eq!(nodes[0].taint_labels.len(), 2);
    }

    #[spec("SEQ-004")]
    #[test]
    fn sequence_numbers_increase_monotonically() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        builder.record_action("a", 1, vec![]);
        builder.record_action("b", 2, vec![]);
        builder.record_action("c", 3, vec![]);
        let nodes = builder.graph().nodes();
        assert!(nodes[0].sequence_number < nodes[1].sequence_number);
        assert!(nodes[1].sequence_number < nodes[2].sequence_number);
    }

    #[spec("SEQ-004")]
    #[test]
    fn graph_count_reflects_each_added_action() {
        let mut builder = SequenceGraphBuilder::new("session-1");
        builder.record_action("read_file", 100, vec!["internal".into()]);
        let count_before = builder.graph().node_count();
        builder.record_action("write_file", 200, vec![]);
        let count_after = builder.graph().node_count();
        assert_eq!(count_after, count_before + 1);
    }
}
