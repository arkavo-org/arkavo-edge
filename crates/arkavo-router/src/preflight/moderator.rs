//! Pre-flight moderation using TØR-G boolean circuits

use dashmap::DashMap;
use torg_core::{evaluate_into, Graph};
use torg_serde::from_bytes;

use super::circuit::CompiledCircuit;
use super::features::PreflightFeature;
use super::result::ModerationResult;

/// Policy identifier for circuit registration
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PolicyId(pub String);

impl PolicyId {
    /// Create a new policy identifier
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for PolicyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Pre-flight moderation using TØR-G boolean circuits
///
/// Evaluates registered circuits against raw request text before LLM inference.
/// Any circuit returning false blocks the request.
pub struct PreflightModerator {
    circuits: DashMap<PolicyId, CompiledCircuit>,
}

impl PreflightModerator {
    /// Create empty moderator
    #[must_use]
    pub fn new() -> Self {
        Self {
            circuits: DashMap::new(),
        }
    }

    /// Register circuit from serialized bytes
    ///
    /// # Errors
    /// Returns error if bytes cannot be deserialized as a valid circuit
    pub fn register(
        &self,
        id: PolicyId,
        bytes: &[u8],
        features: Vec<PreflightFeature>,
    ) -> Result<(), torg_serde::SerdeError> {
        let graph = from_bytes(bytes)?;
        let circuit = CompiledCircuit::new(graph, features);
        self.circuits.insert(id, circuit);
        Ok(())
    }

    /// Register circuit from Graph directly
    pub fn register_graph(&self, id: PolicyId, graph: Graph, features: Vec<PreflightFeature>) {
        let circuit = CompiledCircuit::new(graph, features);
        self.circuits.insert(id, circuit);
    }

    /// Unregister a circuit
    ///
    /// Returns true if the circuit was found and removed
    pub fn unregister(&self, id: &PolicyId) -> bool {
        self.circuits.remove(id).is_some()
    }

    /// Check if any circuits are registered
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.circuits.is_empty()
    }

    /// Number of registered circuits
    #[must_use]
    pub fn len(&self) -> usize {
        self.circuits.len()
    }

    /// Evaluate all circuits against input text
    ///
    /// Returns `Allow` if all circuits pass, or `Block` with the first failing policy
    #[must_use]
    pub fn check(&self, input: &str) -> ModerationResult {
        if self.circuits.is_empty() {
            return ModerationResult::Allow;
        }

        for entry in self.circuits.iter() {
            let policy_id = entry.key();
            let circuit = entry.value();

            // Lock buffers
            let mut input_buf = circuit
                .input_buffer
                .lock()
                .expect("input buffer lock poisoned");
            let mut output_buf = circuit
                .output_buffer
                .lock()
                .expect("output buffer lock poisoned");

            // Extract features
            let mut feature_values = Vec::with_capacity(circuit.feature_map.len());
            for (i, feature) in circuit.feature_map.iter().enumerate() {
                let value = feature.extract(input);
                if i < input_buf.len() {
                    input_buf[i] = value;
                }
                feature_values.push((feature.name(), value));
            }

            // Evaluate circuit
            evaluate_into(&circuit.graph, &input_buf, &mut output_buf);

            // Release input buffer early
            drop(input_buf);

            // Check outputs (any false = policy violation)
            for (i, &output) in output_buf.iter().enumerate() {
                if !output {
                    tracing::warn!(
                        policy = %policy_id.0,
                        output_index = i,
                        "Pre-flight moderation blocked request"
                    );
                    return ModerationResult::block_with_features(
                        &policy_id.0,
                        format!("Policy violated (output {i})"),
                        feature_values,
                    );
                }
            }

            tracing::debug!(
                policy = %policy_id.0,
                "Pre-flight check passed"
            );
        }

        ModerationResult::Allow
    }
}

impl Default for PreflightModerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{BoolOp, Node, Source};
    use torg_serde::to_bytes;

    /// Build a simple NOT circuit: output = NOT(input0)
    /// This blocks when input0 is true
    fn build_not_circuit() -> Graph {
        Graph {
            inputs: vec![0],
            nodes: vec![
                // Node 1: NOR(0, 0) = NOT(input0)
                Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0)),
            ],
            outputs: vec![1],
        }
    }

    /// Build an OR circuit: output = input0 OR input1
    fn build_or_circuit() -> Graph {
        Graph {
            inputs: vec![0, 1],
            nodes: vec![Node::new(2, BoolOp::Or, Source::Id(0), Source::Id(1))],
            outputs: vec![2],
        }
    }

    #[test]
    fn test_empty_moderator_allows() {
        let moderator = PreflightModerator::new();
        assert!(moderator.is_empty());
        assert!(moderator.check("any input").is_allowed());
    }

    #[test]
    fn test_register_and_unregister() {
        let moderator = PreflightModerator::new();
        let graph = build_or_circuit();
        let bytes = to_bytes(&graph);

        moderator
            .register(
                PolicyId::new("test"),
                &bytes,
                vec![
                    PreflightFeature::InputContainsPII,
                    PreflightFeature::InputContainsSQLKeywords,
                ],
            )
            .unwrap();

        assert!(!moderator.is_empty());
        assert_eq!(moderator.len(), 1);

        assert!(moderator.unregister(&PolicyId::new("test")));
        assert!(moderator.is_empty());
    }

    #[test]
    fn test_pii_ssn_blocked() {
        let moderator = PreflightModerator::new();

        // NOT(PII) - blocks when PII is detected
        let graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("block_pii"),
            graph,
            vec![PreflightFeature::InputContainsPII],
        );

        // SSN should be blocked
        let result = moderator.check("My SSN is 123-45-6789");
        assert!(result.is_blocked());
        if let ModerationResult::Block { policy_id, .. } = result {
            assert_eq!(policy_id, "block_pii");
        }
    }

    #[test]
    fn test_pii_credit_card_blocked() {
        let moderator = PreflightModerator::new();

        // NOT(PII) - blocks when PII is detected
        let graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("block_pii"),
            graph,
            vec![PreflightFeature::InputContainsPII],
        );

        // Credit card should be blocked
        let result = moderator.check("Card: 4111-1111-1111-1111");
        assert!(result.is_blocked());
    }

    #[test]
    fn test_clean_input_passes() {
        let moderator = PreflightModerator::new();

        // NOT(PII) - blocks when PII is detected
        let graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("block_pii"),
            graph,
            vec![PreflightFeature::InputContainsPII],
        );

        // Clean input should pass
        let result = moderator.check("What is the weather?");
        assert!(result.is_allowed());
    }

    #[test]
    fn test_sql_injection_blocked() {
        let moderator = PreflightModerator::new();

        // NOT(SQL) - blocks when SQL keywords detected
        let graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("block_injection"),
            graph,
            vec![PreflightFeature::InputContainsSQLKeywords],
        );

        let result = moderator.check("DROP TABLE users;");
        assert!(result.is_blocked());
        if let ModerationResult::Block { policy_id, .. } = result {
            assert_eq!(policy_id, "block_injection");
        }
    }

    #[test]
    fn test_shell_command_blocked() {
        let moderator = PreflightModerator::new();

        // NOT(Shell) - blocks when shell commands detected
        let graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("block_shell"),
            graph,
            vec![PreflightFeature::InputContainsShellCommands],
        );

        let result = moderator.check("Run sudo rm -rf /");
        assert!(result.is_blocked());
        if let ModerationResult::Block { policy_id, .. } = result {
            assert_eq!(policy_id, "block_shell");
        }
    }

    #[test]
    fn test_multiple_circuits_first_violation_wins() {
        let moderator = PreflightModerator::new();

        // Register two policies
        let not_graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("policy_1"),
            not_graph.clone(),
            vec![PreflightFeature::InputContainsPII],
        );

        moderator.register_graph(
            PolicyId::new("policy_2"),
            not_graph,
            vec![PreflightFeature::InputContainsSQLKeywords],
        );

        // SQL should be blocked by policy_2
        let result = moderator.check("DROP TABLE users;");
        assert!(result.is_blocked());
    }

    #[test]
    fn test_length_threshold() {
        let moderator = PreflightModerator::new();

        // NOT(LengthExceeds) - blocks when input is too long
        let graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("length_limit"),
            graph,
            vec![PreflightFeature::InputLengthExceedsThreshold(20)],
        );

        // Short input should pass
        assert!(moderator.check("Short").is_allowed());

        // Long input should be blocked
        let result = moderator.check("This is a very long input that exceeds the threshold");
        assert!(result.is_blocked());
    }

    #[test]
    fn test_feature_values_in_block() {
        let moderator = PreflightModerator::new();

        let graph = build_not_circuit();

        moderator.register_graph(
            PolicyId::new("test"),
            graph,
            vec![PreflightFeature::InputContainsPII],
        );

        let result = moderator.check("SSN: 123-45-6789");
        if let ModerationResult::Block { feature_values, .. } = result {
            assert_eq!(feature_values.len(), 1);
            assert_eq!(feature_values[0].0, "InputContainsPII");
            assert!(feature_values[0].1);
        } else {
            panic!("Expected Block result");
        }
    }
}
