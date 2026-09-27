//! Directed action graph for one session (SEQ-004).
//!
//! Nodes are tool calls; an edge means the output of one call reached the
//! input of another. A per-call anomaly check can only ask "is this call
//! normal"; the graph is what lets a later check ask "is this *sequence*
//! normal", which is where decomposition attacks become visible.

use std::collections::HashMap;

use crate::taint::TaintSet;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Identifier of a node within one session's graph.
pub type NodeId = String;

/// Nodes one session's graph will hold.
///
/// Bounded for the same reason as the label and hop limits in [`crate::taint`]:
/// the count is attacker-drivable. An agent that issues cheap calls in a loop
/// would otherwise grow this without limit. Forensic depth is what the bound
/// costs; it costs no accuracy in the taint a gate reads, because callers that
/// need session-wide taint accumulate it as they go rather than re-deriving it
/// from the graph.
pub const MAX_NODES: usize = 4096;

/// Rejections that leave the graph untouched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// An edge was requested from a node this graph does not hold. Accepting
    /// it would produce a graph whose data-flow edges are not reconstructable.
    #[error("unknown input node: {0}")]
    UnknownInput(NodeId),
    /// The graph is at [`MAX_NODES`]. The call still happened and its taint
    /// still counts; only the forensic record stops growing.
    #[error("sequence graph is full at {MAX_NODES} nodes")]
    Full,
}

/// One tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceNode {
    pub id: NodeId,
    pub tool_name: String,
    /// Digest of the call parameters, so two calls correlate only when their
    /// parameters really were identical.
    pub params_hash: String,
    /// Taint the call handled, after inputs were merged in.
    pub taint: TaintSet,
    /// Nodes whose output flowed into this one.
    pub inputs: Vec<NodeId>,
}

/// Accumulates a session's graph as calls complete.
#[derive(Debug, Clone, Default)]
pub struct SequenceGraphBuilder {
    nodes: Vec<SequenceNode>,
    index: HashMap<NodeId, usize>,
    /// Calls refused at [`MAX_NODES`]. Non-zero means this graph is a prefix of
    /// the session, which an auditor has to be able to see.
    truncated_nodes: u32,
}

impl SequenceGraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn nodes(&self) -> &[SequenceNode] {
        &self.nodes
    }

    pub fn node(&self, id: &str) -> Option<&SequenceNode> {
        self.index.get(id).and_then(|i| self.nodes.get(*i))
    }

    /// Calls this graph could not record. Non-zero means it is a prefix.
    pub fn truncated_nodes(&self) -> u32 {
        self.truncated_nodes
    }

    /// Every data-flow edge as `(from, to)`.
    pub fn edges(&self) -> impl Iterator<Item = (&str, &str)> {
        self.nodes.iter().flat_map(|node| {
            node.inputs
                .iter()
                .map(move |input| (input.as_str(), node.id.as_str()))
        })
    }

    /// Union of the taint held by the named nodes. This is what flows into the
    /// next call, so it is computed from the graph rather than trusted from
    /// the caller.
    pub fn taint_flowing_from(&self, inputs: &[NodeId]) -> Result<TaintSet, GraphError> {
        let mut flowing = TaintSet::new();
        for input in inputs {
            let node = self
                .node(input)
                .ok_or_else(|| GraphError::UnknownInput(input.clone()))?;
            flowing.merge(&node.taint);
        }
        Ok(flowing)
    }

    /// Append a completed call. The node is built and validated in full before
    /// anything is mutated, so a rejected call leaves the graph exactly as it
    /// was — a half-added node would corrupt every later reconstruction.
    pub fn push(
        &mut self,
        tool_name: &str,
        params: &Value,
        inputs: &[NodeId],
        taint: TaintSet,
    ) -> Result<NodeId, GraphError> {
        for input in inputs {
            if !self.index.contains_key(input) {
                return Err(GraphError::UnknownInput(input.clone()));
            }
        }
        if self.nodes.len() >= MAX_NODES {
            self.truncated_nodes = self.truncated_nodes.saturating_add(1);
            return Err(GraphError::Full);
        }
        let node = SequenceNode {
            id: format!("n{}", self.nodes.len()),
            tool_name: tool_name.to_string(),
            params_hash: params_digest(params),
            taint,
            inputs: inputs.to_vec(),
        };
        let id = node.id.clone();
        // Node first, index second. If a write between the two ever failed, a
        // reader would find a node with no index entry — which reads as an
        // unknown input and fails closed — rather than an index entry pointing
        // past the end of the vector.
        let position = self.nodes.len();
        self.nodes.push(node);
        self.index.insert(id.clone(), position);
        Ok(id)
    }
}

/// SHA-256 over the canonical JSON encoding of a call's parameters.
///
/// `serde_json::Value` orders object keys, so the encoding is stable across
/// runs and two structurally equal parameter sets digest alike.
pub fn params_digest(params: &Value) -> String {
    let canonical = serde_json::to_vec(params).unwrap_or_default();
    let digest = Sha256::digest(&canonical);
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_classification::{DataCategory, SensitivityLevel};
    use crate::taint::TaintLabel;
    use serde_json::json;

    fn tainted(source: &str, level: SensitivityLevel) -> TaintSet {
        TaintSet::from_label(TaintLabel::new(source, [DataCategory::Internal], level))
    }

    #[test]
    fn a_new_graph_is_empty() {
        let graph = SequenceGraphBuilder::new();
        assert!(graph.is_empty());
        assert_eq!(graph.edges().count(), 0);
    }

    #[test]
    fn push_returns_a_node_that_can_be_looked_up() {
        let mut graph = SequenceGraphBuilder::new();

        let id = graph
            .push(
                "read_file",
                &json!({"path": "/etc/hosts"}),
                &[],
                TaintSet::new(),
            )
            .expect("no inputs to validate");

        let node = graph.node(&id).expect("node stored");
        assert_eq!(node.tool_name, "read_file");
        assert!(node.inputs.is_empty());
    }

    #[test]
    fn edges_connect_output_to_input() {
        let mut graph = SequenceGraphBuilder::new();
        let read = graph
            .push("read_file", &json!({}), &[], TaintSet::new())
            .expect("root node");
        let post = graph
            .push("http_post", &json!({}), &[read.clone()], TaintSet::new())
            .expect("known input");

        let edges: Vec<(String, String)> = graph
            .edges()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();

        assert_eq!(edges, vec![(read, post)]);
    }

    #[test]
    fn an_unknown_input_is_rejected_and_the_graph_is_unchanged() {
        let mut graph = SequenceGraphBuilder::new();
        graph
            .push("read_file", &json!({}), &[], TaintSet::new())
            .expect("root node");

        let err = graph
            .push(
                "http_post",
                &json!({}),
                &["n99".to_string()],
                TaintSet::new(),
            )
            .expect_err("unknown input rejected");

        assert_eq!(err, GraphError::UnknownInput("n99".to_string()));
        assert_eq!(graph.len(), 1);
    }

    #[test]
    fn taint_flows_from_named_inputs() {
        let mut graph = SequenceGraphBuilder::new();
        let a = graph
            .push(
                "read_file",
                &json!({}),
                &[],
                tainted("file:a", SensitivityLevel::Internal),
            )
            .expect("root");
        let b = graph
            .push(
                "read_vault",
                &json!({}),
                &[],
                tainted("tool:vault", SensitivityLevel::Restricted),
            )
            .expect("root");

        let flowing = graph.taint_flowing_from(&[a, b]).expect("known inputs");

        assert_eq!(flowing.sensitivity(), SensitivityLevel::Restricted);
        assert_eq!(flowing.len(), 2);
    }

    #[test]
    fn taint_from_an_unknown_node_is_an_error_not_an_empty_set() {
        let graph = SequenceGraphBuilder::new();

        let err = graph
            .taint_flowing_from(&["n0".to_string()])
            .expect_err("unknown node");

        assert_eq!(err, GraphError::UnknownInput("n0".to_string()));
    }

    #[test]
    fn identical_parameters_digest_identically() {
        assert_eq!(
            params_digest(&json!({"a": 1, "b": 2})),
            params_digest(&json!({"b": 2, "a": 1}))
        );
    }

    #[test]
    fn different_parameters_digest_differently() {
        assert_ne!(
            params_digest(&json!({"a": 1})),
            params_digest(&json!({"a": 2}))
        );
    }

    #[test]
    fn the_graph_stops_growing_at_the_node_bound() {
        let mut graph = SequenceGraphBuilder::new();
        for _ in 0..MAX_NODES {
            graph
                .push("read", &json!({}), &[], TaintSet::new())
                .expect("under the bound");
        }

        let refused = graph.push("read", &json!({}), &[], TaintSet::new());

        assert_eq!(refused, Err(GraphError::Full));
        assert_eq!(graph.len(), MAX_NODES);
        assert_eq!(graph.truncated_nodes(), 1);
    }

    #[test]
    fn a_full_graph_still_answers_for_what_it_holds() {
        // The bound costs forensic depth, not the ability to read the prefix.
        let mut graph = SequenceGraphBuilder::new();
        let first = graph
            .push("read", &json!({"p": 1}), &[], TaintSet::new())
            .expect("first node");
        for _ in 1..MAX_NODES {
            graph
                .push("read", &json!({}), &[], TaintSet::new())
                .expect("under the bound");
        }
        let _ = graph.push("read", &json!({}), &[], TaintSet::new());

        assert!(graph.node(&first).is_some());
    }
}
