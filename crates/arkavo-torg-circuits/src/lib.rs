//! Generic boolean circuit evaluation with pre-allocated buffers
//!
//! This crate provides a generic `CompiledCircuit<F>` that can be used for
//! policy evaluation with any feature type implementing `CircuitFeature`.

use parking_lot::Mutex;
use torg_core::{Graph, evaluate_into};

// Re-export for convenience
pub use torg_core;
pub use torg_serde;

/// Trait for features that can be extracted for circuit input
///
/// Implement this trait for your feature enum to use `CompiledCircuit<F>`.
pub trait CircuitFeature: Clone + Send + Sync {
    /// The input type from which features are extracted
    type Input: ?Sized;

    /// Extract a boolean value from the input
    fn extract(&self, input: &Self::Input) -> bool;

    /// Get a human-readable name for debugging/logging
    fn name(&self) -> String;
}

/// Generic compiled circuit with pre-allocated buffers
///
/// Uses `parking_lot::Mutex` for better performance on uncontended locks.
/// Buffers are pre-allocated at construction time for zero-allocation evaluation.
pub struct CompiledCircuit<F: CircuitFeature> {
    /// The underlying boolean circuit graph
    pub graph: Graph,
    /// Feature extractors mapped to circuit inputs
    pub feature_map: Vec<F>,
    /// Pre-allocated input buffer (protected by mutex for thread safety)
    input_buffer: Mutex<Vec<bool>>,
    /// Pre-allocated output buffer (protected by mutex for thread safety)
    output_buffer: Mutex<Vec<bool>>,
}

impl<F: CircuitFeature> CompiledCircuit<F> {
    /// Create a new compiled circuit with validation
    ///
    /// # Panics
    ///
    /// Panics if `feature_map.len()` does not match the number of circuit inputs.
    /// This is intentional to catch configuration errors at construction time
    /// rather than causing silent failures during evaluation.
    #[must_use]
    pub fn new(graph: Graph, feature_map: Vec<F>) -> Self {
        let expected_inputs = graph
            .inputs
            .iter()
            .max()
            .map(|&m| m as usize + 1)
            .unwrap_or(0);

        assert_eq!(
            feature_map.len(),
            expected_inputs,
            "Feature count ({}) must match circuit input count ({})",
            feature_map.len(),
            expected_inputs
        );

        let output_len = graph.outputs.len();

        Self {
            graph,
            feature_map,
            input_buffer: Mutex::new(vec![false; expected_inputs]),
            output_buffer: Mutex::new(vec![false; output_len]),
        }
    }

    /// Evaluate circuit with given input
    ///
    /// Returns `Some(index)` if any output is false (policy violation),
    /// where `index` is the first failing output. Returns `None` if all
    /// outputs are true (policy passes).
    pub fn evaluate(&self, input: &F::Input) -> Option<usize> {
        let mut input_buf = self.input_buffer.lock();
        let mut output_buf = self.output_buffer.lock();

        // Extract features into input buffer
        for (i, feature) in self.feature_map.iter().enumerate() {
            input_buf[i] = feature.extract(input);
        }

        // Evaluate circuit (nanosecond operation)
        evaluate_into(&self.graph, &input_buf, &mut output_buf);

        // Release input buffer early
        drop(input_buf);

        // Find first failing output (false = policy violation)
        output_buf.iter().position(|&v| !v)
    }

    /// Evaluate circuit and collect feature values for debugging
    ///
    /// Returns a tuple of:
    /// - `Option<usize>`: First failing output index, or None if passed
    /// - `Vec<(String, bool)>`: Feature names and their extracted values
    pub fn evaluate_with_features(&self, input: &F::Input) -> (Option<usize>, Vec<(String, bool)>) {
        let mut input_buf = self.input_buffer.lock();
        let mut output_buf = self.output_buffer.lock();

        // Extract features and collect debug info
        let mut feature_values = Vec::with_capacity(self.feature_map.len());
        for (i, feature) in self.feature_map.iter().enumerate() {
            let value = feature.extract(input);
            input_buf[i] = value;
            feature_values.push((feature.name(), value));
        }

        // Evaluate circuit
        evaluate_into(&self.graph, &input_buf, &mut output_buf);

        // Release input buffer early
        drop(input_buf);

        // Find first failing output and release output buffer
        let failing = output_buf.iter().position(|&v| !v);
        drop(output_buf);

        (failing, feature_values)
    }

    /// Get the number of circuit inputs
    #[must_use]
    pub fn input_count(&self) -> usize {
        self.feature_map.len()
    }

    /// Get the number of circuit outputs
    #[must_use]
    pub fn output_count(&self) -> usize {
        self.graph.outputs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{BoolOp, Node, Source};

    /// Simple feature for testing
    #[derive(Clone)]
    struct TestFeature {
        name: String,
        value: bool,
    }

    impl CircuitFeature for TestFeature {
        type Input = ();

        fn extract(&self, _input: &Self::Input) -> bool {
            self.value
        }

        fn name(&self) -> String {
            self.name.clone()
        }
    }

    fn build_not_circuit() -> Graph {
        Graph {
            inputs: vec![0],
            nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))],
            outputs: vec![1],
        }
    }

    fn build_or_circuit() -> Graph {
        Graph {
            inputs: vec![0, 1],
            nodes: vec![Node::new(2, BoolOp::Or, Source::Id(0), Source::Id(1))],
            outputs: vec![2],
        }
    }

    #[test]
    fn test_not_circuit_pass() {
        let graph = build_not_circuit();
        let features = vec![TestFeature {
            name: "input0".into(),
            value: false,
        }];

        let circuit = CompiledCircuit::new(graph, features);
        assert!(circuit.evaluate(&()).is_none()); // NOT(false) = true, passes
    }

    #[test]
    fn test_not_circuit_fail() {
        let graph = build_not_circuit();
        let features = vec![TestFeature {
            name: "input0".into(),
            value: true,
        }];

        let circuit = CompiledCircuit::new(graph, features);
        assert_eq!(circuit.evaluate(&()), Some(0)); // NOT(true) = false, fails at output 0
    }

    #[test]
    fn test_or_circuit() {
        let graph = build_or_circuit();
        let features = vec![
            TestFeature {
                name: "a".into(),
                value: false,
            },
            TestFeature {
                name: "b".into(),
                value: true,
            },
        ];

        let circuit = CompiledCircuit::new(graph, features);
        assert!(circuit.evaluate(&()).is_none()); // false OR true = true, passes
    }

    #[test]
    fn test_evaluate_with_features() {
        let graph = build_not_circuit();
        let features = vec![TestFeature {
            name: "test_feature".into(),
            value: true,
        }];

        let circuit = CompiledCircuit::new(graph, features);
        let (result, feature_values) = circuit.evaluate_with_features(&());

        assert_eq!(result, Some(0));
        assert_eq!(feature_values.len(), 1);
        assert_eq!(feature_values[0].0, "test_feature");
        assert!(feature_values[0].1);
    }

    #[test]
    #[should_panic(expected = "Feature count (1) must match circuit input count (2)")]
    fn test_mismatched_features_panics() {
        let graph = build_or_circuit(); // 2 inputs
        let features = vec![TestFeature {
            name: "only_one".into(),
            value: false,
        }]; // Only 1 feature

        let _ = CompiledCircuit::new(graph, features);
    }

    #[test]
    fn test_input_output_counts() {
        let graph = build_or_circuit();
        let features = vec![
            TestFeature {
                name: "a".into(),
                value: false,
            },
            TestFeature {
                name: "b".into(),
                value: false,
            },
        ];

        let circuit = CompiledCircuit::new(graph, features);
        assert_eq!(circuit.input_count(), 2);
        assert_eq!(circuit.output_count(), 1);
    }
}
