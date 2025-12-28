//! TØR-G circuit check
//!
//! Fast-path policy gate using boolean circuits for sub-microsecond evaluation.
//! Circuits are registered dynamically and can be hot-reloaded.

use super::traits::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::{CheckSeverity, VerificationEvidence};
use crate::features::FeatureId;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Mutex;
use std::time::Instant;
use torg_core::{evaluate_into, Graph};

/// Policy identifier for circuit registration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PolicyId(pub String);

impl PolicyId {
    /// Create a new policy ID.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Compiled circuit with pre-allocated buffers for zero-allocation evaluation.
pub struct CompiledCircuit {
    graph: Graph,
    feature_map: Vec<FeatureId>,
    /// Pre-allocated input buffer (protected by mutex for thread safety)
    input_buffer: Mutex<Vec<bool>>,
    /// Pre-allocated output buffer
    output_buffer: Mutex<Vec<bool>>,
}

impl CompiledCircuit {
    /// Create a new compiled circuit with pre-allocated buffers.
    #[must_use]
    pub fn new(graph: Graph, feature_map: Vec<FeatureId>) -> Self {
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

    /// Evaluate the circuit with the given input.
    ///
    /// Returns `Some(i)` if output `i` is false (policy violation),
    /// or `None` if all outputs are true (policy passes).
    fn evaluate(&self, input: &VerificationInput) -> Option<usize> {
        let mut input_buf = self.input_buffer.lock().unwrap();
        let mut output_buf = self.output_buffer.lock().unwrap();

        // Extract features into input buffer
        for (i, feature) in self.feature_map.iter().enumerate() {
            if i < input_buf.len() {
                input_buf[i] = feature.extract(input);
            }
        }

        // Evaluate circuit (nanoseconds)
        evaluate_into(&self.graph, &input_buf, &mut output_buf);

        // Release input buffer early - no longer needed
        drop(input_buf);

        // Find first failing output
        output_buf.iter().position(|&v| !v)
    }
}

/// Fast-path policy gate using TØR-G boolean circuits.
///
/// Circuits can be registered dynamically and evaluated in nanoseconds.
/// Each circuit maps boolean features extracted from `VerificationInput`
/// to policy decisions.
pub struct CircuitCheck {
    priority: u32,
    circuits: DashMap<PolicyId, CompiledCircuit>,
}

impl CircuitCheck {
    /// Create a new circuit check with default priority (5).
    ///
    /// Priority 5 runs before SchemaCheck (10).
    #[must_use]
    pub fn new() -> Self {
        Self {
            priority: 5,
            circuits: DashMap::new(),
        }
    }

    /// Create with custom priority.
    #[must_use]
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Register a circuit from serialized bytes.
    ///
    /// # Errors
    ///
    /// Returns error if deserialization fails.
    pub fn register(
        &self,
        id: PolicyId,
        bytes: &[u8],
        features: Vec<FeatureId>,
    ) -> Result<(), torg_serde::SerdeError> {
        let graph = torg_serde::from_bytes(bytes)?;
        let circuit = CompiledCircuit::new(graph, features);
        self.circuits.insert(id, circuit);
        Ok(())
    }

    /// Register a circuit from a pre-built graph.
    pub fn register_graph(&self, id: PolicyId, graph: Graph, features: Vec<FeatureId>) {
        let circuit = CompiledCircuit::new(graph, features);
        self.circuits.insert(id, circuit);
    }

    /// Unregister a circuit (hot-reload support).
    ///
    /// Returns true if the circuit was found and removed.
    pub fn unregister(&self, id: &PolicyId) -> bool {
        self.circuits.remove(id).is_some()
    }

    /// Check if any circuits are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.circuits.is_empty()
    }

    /// Get the number of registered circuits.
    #[must_use]
    pub fn len(&self) -> usize {
        self.circuits.len()
    }
}

impl Default for CircuitCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VerificationCheck for CircuitCheck {
    fn id(&self) -> &'static str {
        "circuit"
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    fn is_applicable(&self, _input: &VerificationInput) -> bool {
        // Skip if no circuits registered
        !self.circuits.is_empty()
    }

    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let start = Instant::now();

        // Evaluate all circuits
        for entry in &self.circuits {
            let policy_id = entry.key();
            let circuit = entry.value();

            if let Some(failing_output) = circuit.evaluate(input) {
                let latency = start.elapsed().as_millis() as u64;
                let evidence = VerificationEvidence::failed(
                    self.id(),
                    &format!(
                        "Policy '{}' violated (output {} is false)",
                        policy_id.0, failing_output
                    ),
                    CheckSeverity::Error,
                    latency,
                );
                tracing::warn!(
                    check = "circuit",
                    policy = %policy_id.0,
                    output = failing_output,
                    latency_ms = latency,
                    "Circuit check failed"
                );
                return CheckResult::Fail(evidence);
            }
        }

        let latency = start.elapsed().as_millis() as u64;
        tracing::debug!(
            check = "circuit",
            circuits = self.circuits.len(),
            latency_ms = latency,
            "All circuits passed"
        );

        CheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::tool_parser::ParsedToolCall;
    use arkavo_llm::ProviderResponse;
    use torg_core::{BoolOp, Node, Source};
    use torg_serde::to_bytes;

    fn make_response(content: &str, tool_calls: Vec<ParsedToolCall>) -> ProviderResponse {
        ProviderResponse {
            content: content.to_string(),
            reasoning_content: None,
            tool_calls,
            finish_reason: None,
        }
    }

    fn make_simple_or_graph() -> Graph {
        // Output = input0 OR input1
        Graph {
            inputs: vec![0, 1],
            nodes: vec![Node::new(2, BoolOp::Or, Source::Id(0), Source::Id(1))],
            outputs: vec![2],
        }
    }

    fn make_and_graph() -> Graph {
        // AND(a,b) = NOR(NOR(a,a), NOR(b,b)) = NOR(NOT(a), NOT(b))
        // If we have OR, we can build AND as: NOT(OR(NOT(a), NOT(b)))
        // Using NOR: NOR(a,a) = NOT(a), so AND(a,b) = NOR(NOR(a,a), NOR(b,b))
        Graph {
            inputs: vec![0, 1],
            nodes: vec![
                Node::new(2, BoolOp::Nor, Source::Id(0), Source::Id(0)), // NOT(a)
                Node::new(3, BoolOp::Nor, Source::Id(1), Source::Id(1)), // NOT(b)
                Node::new(4, BoolOp::Nor, Source::Id(2), Source::Id(3)), // NOR(NOT(a), NOT(b)) = AND(a,b)
            ],
            outputs: vec![4],
        }
    }

    #[test]
    fn test_register_circuit() {
        let check = CircuitCheck::new();
        let graph = make_simple_or_graph();
        let bytes = to_bytes(&graph);

        check
            .register(
                PolicyId::new("test"),
                &bytes,
                vec![FeatureId::HasToolCalls, FeatureId::ResponseIsEmpty],
            )
            .unwrap();

        assert!(!check.is_empty());
        assert_eq!(check.len(), 1);
    }

    #[test]
    fn test_unregister_circuit() {
        let check = CircuitCheck::new();
        let graph = make_simple_or_graph();

        check.register_graph(
            PolicyId::new("test"),
            graph,
            vec![FeatureId::HasToolCalls, FeatureId::ResponseIsEmpty],
        );

        assert!(!check.is_empty());
        assert!(check.unregister(&PolicyId::new("test")));
        assert!(check.is_empty());
    }

    #[tokio::test]
    async fn test_circuit_passes_or() {
        let check = CircuitCheck::new();
        let graph = make_simple_or_graph();

        // Policy: HasToolCalls OR ResponseHasCode
        check.register_graph(
            PolicyId::new("allow_tools_or_code"),
            graph,
            vec![FeatureId::HasToolCalls, FeatureId::ResponseHasCode],
        );

        // Response with code should pass (false OR true = true)
        let input = VerificationInput::new(
            "test".into(),
            make_response("```rust\nfn main() {}\n```", vec![]),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_pass());
    }

    #[tokio::test]
    async fn test_circuit_fails() {
        let check = CircuitCheck::new();
        let graph = make_simple_or_graph();

        // Policy: HasToolCalls OR ResponseHasCode
        check.register_graph(
            PolicyId::new("require_tools_or_code"),
            graph,
            vec![FeatureId::HasToolCalls, FeatureId::ResponseHasCode],
        );

        // Response with no tools and no code should fail (false OR false = false)
        let input = VerificationInput::new(
            "test".into(),
            make_response("Just plain text", vec![]),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_fail());
        let evidence = result.evidence().unwrap();
        assert!(evidence.description.contains("require_tools_or_code"));
    }

    #[tokio::test]
    async fn test_circuit_with_tools() {
        let check = CircuitCheck::new();
        let graph = make_simple_or_graph();

        // Policy: HasToolCalls OR ResponseHasCode
        check.register_graph(
            PolicyId::new("test"),
            graph,
            vec![FeatureId::HasToolCalls, FeatureId::ResponseHasCode],
        );

        // Response with tools should pass (true OR false = true)
        let input = VerificationInput::new(
            "test".into(),
            make_response(
                "Using tool",
                vec![ParsedToolCall {
                    tool_name: "search".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_pass());
    }

    #[tokio::test]
    async fn test_and_circuit() {
        let check = CircuitCheck::new();
        let graph = make_and_graph();

        // Policy: HasToolCalls AND ResponseHasCode (both must be true)
        check.register_graph(
            PolicyId::new("require_both"),
            graph,
            vec![FeatureId::HasToolCalls, FeatureId::ResponseHasCode],
        );

        // Only tools, no code - should fail
        let input = VerificationInput::new(
            "test".into(),
            make_response(
                "Using tool",
                vec![ParsedToolCall {
                    tool_name: "search".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_fail());

        // Both tools and code - should pass
        let input_both = VerificationInput::new(
            "test".into(),
            make_response(
                "```code```",
                vec![ParsedToolCall {
                    tool_name: "search".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        );

        let result = check.verify(&input_both).await;
        assert!(result.is_pass());
    }

    #[tokio::test]
    async fn test_skip_when_no_circuits() {
        let check = CircuitCheck::new();

        let input = VerificationInput::new(
            "test".into(),
            make_response("Hello", vec![]),
            vec![],
        );

        // is_applicable should return false
        assert!(!check.is_applicable(&input));
    }

    #[tokio::test]
    async fn test_multiple_circuits() {
        let check = CircuitCheck::new();

        // First circuit: require tools or code
        check.register_graph(
            PolicyId::new("tools_or_code"),
            make_simple_or_graph(),
            vec![FeatureId::HasToolCalls, FeatureId::ResponseHasCode],
        );

        // Second circuit: require non-empty response
        let non_empty_graph = Graph {
            inputs: vec![0],
            nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))], // NOT(ResponseIsEmpty)
            outputs: vec![1],
        };
        check.register_graph(
            PolicyId::new("non_empty"),
            non_empty_graph,
            vec![FeatureId::ResponseIsEmpty],
        );

        assert_eq!(check.len(), 2);

        // Response with code and non-empty should pass both
        let input = VerificationInput::new(
            "test".into(),
            make_response("```code```", vec![]),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_pass());
    }

    /// Build a circuit that returns true (PASS) unless guest AND write.
    ///
    /// Circuit: NOT(guest AND write)
    /// - Input 0: is_guest
    /// - Input 1: intent_is_write
    /// - Node 2: NOR(0, 0) = NOT(guest)
    /// - Node 3: NOR(1, 1) = NOT(write)
    /// - Node 4: NOR(NOT_guest, NOT_write) = guest AND write
    /// - Node 5: NOR(4, 4) = NOT(guest AND write) = output
    fn make_block_guest_writes_circuit() -> Graph {
        Graph {
            inputs: vec![0, 1],
            nodes: vec![
                Node::new(2, BoolOp::Nor, Source::Id(0), Source::Id(0)), // NOT(guest)
                Node::new(3, BoolOp::Nor, Source::Id(1), Source::Id(1)), // NOT(write)
                Node::new(4, BoolOp::Nor, Source::Id(2), Source::Id(3)), // guest AND write
                Node::new(5, BoolOp::Nor, Source::Id(4), Source::Id(4)), // NOT(guest AND write)
            ],
            outputs: vec![5],
        }
    }

    /// Test the block guest writes circuit logic in isolation.
    ///
    /// Truth table:
    /// | guest | write | guest AND write | NOT(...) | Result |
    /// |-------|-------|-----------------|----------|--------|
    /// | false | false | false           | true     | PASS   |
    /// | false | true  | false           | true     | PASS   |
    /// | true  | false | false           | true     | PASS   |
    /// | true  | true  | true            | false    | BLOCK  |
    #[test]
    fn test_block_guest_writes_logic() {
        use torg_core::evaluate_into;

        let graph = make_block_guest_writes_circuit();
        let mut outputs = [false];

        // guest=false, write=false → should pass (output=true)
        evaluate_into(&graph, &[false, false], &mut outputs);
        assert!(outputs[0], "non-guest + non-write should pass");

        // guest=false, write=true → should pass (output=true)
        evaluate_into(&graph, &[false, true], &mut outputs);
        assert!(outputs[0], "non-guest + write should pass");

        // guest=true, write=false → should pass (output=true)
        evaluate_into(&graph, &[true, false], &mut outputs);
        assert!(outputs[0], "guest + non-write should pass");

        // guest=true, write=true → should block (output=false)
        evaluate_into(&graph, &[true, true], &mut outputs);
        assert!(!outputs[0], "guest + write should be blocked");
    }

    /// Full integration test: block guest writes policy.
    ///
    /// Uses Custom("is_guest") to extract guest status from context JSON.
    #[tokio::test]
    async fn test_circuit_check_blocks_guest_writes() {
        let check = CircuitCheck::new();
        let graph = make_block_guest_writes_circuit();
        let bytes = to_bytes(&graph);

        // Register policy: block when is_guest AND intent_is_write
        check
            .register(
                PolicyId::new("block_guest_writes"),
                &bytes,
                vec![
                    FeatureId::Custom("is_guest".into()),
                    FeatureId::IntentIsWrite,
                ],
            )
            .unwrap();

        // Test case 1: Guest + Write → BLOCKED
        let input_guest_write = VerificationInput::new(
            "test".into(),
            make_response(
                "Writing...",
                vec![ParsedToolCall {
                    tool_name: "file_write".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        )
        .with_context(serde_json::json!({"is_guest": true}));

        let result = check.verify(&input_guest_write).await;
        assert!(
            result.is_fail(),
            "Expected guest write to be blocked, got pass"
        );
        let evidence = result.evidence().unwrap();
        assert!(
            evidence.description.contains("block_guest_writes"),
            "Expected policy name in error"
        );

        // Test case 2: Guest + Read → ALLOWED
        let input_guest_read = VerificationInput::new(
            "test".into(),
            make_response(
                "Reading...",
                vec![ParsedToolCall {
                    tool_name: "file_read".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        )
        .with_context(serde_json::json!({"is_guest": true}));

        let result = check.verify(&input_guest_read).await;
        assert!(result.is_pass(), "Expected guest read to pass");

        // Test case 3: Admin + Write → ALLOWED
        let input_admin_write = VerificationInput::new(
            "test".into(),
            make_response(
                "Writing...",
                vec![ParsedToolCall {
                    tool_name: "file_write".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        )
        .with_context(serde_json::json!({"is_guest": false}));

        let result = check.verify(&input_admin_write).await;
        assert!(result.is_pass(), "Expected admin write to pass");

        // Test case 4: Admin + Read → ALLOWED
        let input_admin_read = VerificationInput::new(
            "test".into(),
            make_response(
                "Reading...",
                vec![ParsedToolCall {
                    tool_name: "file_read".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        )
        .with_context(serde_json::json!({"is_guest": false}));

        let result = check.verify(&input_admin_read).await;
        assert!(result.is_pass(), "Expected admin read to pass");
    }
}
