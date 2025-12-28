//! Compiled circuit with pre-allocated buffers for zero-alloc evaluation

use std::sync::Mutex;

use torg_core::Graph;

use super::features::PreflightFeature;

/// Compiled circuit with pre-allocated buffers for zero-alloc evaluation
pub(super) struct CompiledCircuit {
    pub(crate) graph: Graph,
    pub(crate) feature_map: Vec<PreflightFeature>,
    pub(crate) input_buffer: Mutex<Vec<bool>>,
    pub(crate) output_buffer: Mutex<Vec<bool>>,
}

impl CompiledCircuit {
    /// Create a new compiled circuit with pre-allocated buffers
    pub(super) fn new(graph: Graph, feature_map: Vec<PreflightFeature>) -> Self {
        let input_len = graph
            .inputs
            .iter()
            .max()
            .map(|&m| m as usize + 1)
            .unwrap_or(0);
        let output_len = graph.outputs.len();

        Self {
            graph,
            feature_map,
            input_buffer: Mutex::new(vec![false; input_len]),
            output_buffer: Mutex::new(vec![false; output_len]),
        }
    }
}
