//! Integration tests for TØR-G compiled circuits
//!
//! Validates the full pipeline: Graph build → serialize → deserialize → compile → evaluate

use arkavo_torg_circuits::{CircuitFeature, CompiledCircuit, torg_core, torg_serde};
use std::sync::Arc;
use std::thread;
use torg_core::{BoolOp, Builder, Graph, Node, Source, Token, evaluate};

// -- Feature implementation for integration tests --------------------------

#[derive(Clone)]
struct RequestFeature {
    name: &'static str,
    extractor: fn(&str) -> bool,
}

impl CircuitFeature for RequestFeature {
    type Input = str;

    fn extract(&self, input: &Self::Input) -> bool {
        (self.extractor)(input)
    }

    fn name(&self) -> String {
        self.name.into()
    }
}

fn contains_pii(input: &str) -> bool {
    input.contains("SSN") || input.contains("123-45-6789")
}

fn contains_sql(input: &str) -> bool {
    let upper = input.to_uppercase();
    ["DROP", "SELECT", "DELETE", "INSERT"]
        .iter()
        .any(|kw| upper.contains(kw))
}

fn contains_shell(input: &str) -> bool {
    let lower = input.to_lowercase();
    ["rm ", "sudo ", "curl ", "wget "]
        .iter()
        .any(|cmd| lower.contains(cmd))
}

fn is_too_long(input: &str) -> bool {
    input.len() > 1000
}

// -- Graph builders --------------------------------------------------------

fn build_not_graph() -> Graph {
    Graph {
        inputs: vec![0],
        nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))],
        outputs: vec![1],
    }
}

fn build_and_graph() -> Graph {
    // AND(a, b) = NOR(NOR(a,a), NOR(b,b))
    Graph {
        inputs: vec![0, 1],
        nodes: vec![
            Node::new(2, BoolOp::Nor, Source::Id(0), Source::Id(0)), // NOT a
            Node::new(3, BoolOp::Nor, Source::Id(1), Source::Id(1)), // NOT b
            Node::new(4, BoolOp::Nor, Source::Id(2), Source::Id(3)), // NOR(NOT a, NOT b) = AND
        ],
        outputs: vec![4],
    }
}

/// "Block if PII OR SQL" = NOT(PII OR SQL)
fn build_block_pii_or_sql() -> Graph {
    Graph {
        inputs: vec![0, 1],
        nodes: vec![
            Node::new(2, BoolOp::Or, Source::Id(0), Source::Id(1)), // PII OR SQL
            Node::new(3, BoolOp::Nor, Source::Id(2), Source::Id(2)), // NOT(PII OR SQL)
        ],
        outputs: vec![3],
    }
}

// -- Test: Serialize/Deserialize roundtrip ---------------------------------

#[test]
fn test_graph_serde_roundtrip() {
    let graph = build_block_pii_or_sql();
    let bytes = torg_serde::to_bytes(&graph);
    let restored: Graph = torg_serde::from_bytes(&bytes).expect("deserialize failed");

    assert_eq!(graph.inputs, restored.inputs);
    assert_eq!(graph.outputs, restored.outputs);
    assert_eq!(graph.nodes.len(), restored.nodes.len());

    // Verify evaluation equivalence
    let features = vec![
        RequestFeature {
            name: "pii",
            extractor: contains_pii,
        },
        RequestFeature {
            name: "sql",
            extractor: contains_sql,
        },
    ];

    let orig = CompiledCircuit::new(graph, features.clone());
    let restored = CompiledCircuit::new(restored, features);

    // Clean input: both should pass
    assert!(orig.evaluate("Hello world").is_none());
    assert!(restored.evaluate("Hello world").is_none());

    // PII input: both should block
    assert!(orig.evaluate("My SSN is 123-45-6789").is_some());
    assert!(restored.evaluate("My SSN is 123-45-6789").is_some());
}

// -- Test: Builder token API → compiled circuit ----------------------------

#[test]
fn test_builder_to_circuit() {
    // Build "NOT(input)" using the token-based Builder API
    let mut builder = Builder::new();
    for &token in &[
        Token::InputDecl,
        Token::Id(0),
        Token::NodeStart,
        Token::Id(1),
        Token::Nor,
        Token::Id(0),
        Token::Id(0),
        Token::NodeEnd,
        Token::OutputDecl,
        Token::Id(1),
    ] {
        builder.push(token).unwrap();
    }
    let graph = builder.finish().unwrap();

    let circuit = CompiledCircuit::new(
        graph,
        vec![RequestFeature {
            name: "pii",
            extractor: contains_pii,
        }],
    );

    // NOT(contains_pii("safe")) = NOT(false) = true → passes
    assert!(circuit.evaluate("safe input").is_none());

    // NOT(contains_pii("SSN 123-45-6789")) = NOT(true) = false → blocks
    assert_eq!(circuit.evaluate("SSN 123-45-6789"), Some(0));
}

// -- Test: Multi-policy composition ----------------------------------------

#[test]
fn test_multi_output_circuit() {
    // Two independent NOT gates: output[0] = NOT(input0), output[1] = NOT(input1)
    let graph = Graph {
        inputs: vec![0, 1],
        nodes: vec![
            Node::new(2, BoolOp::Nor, Source::Id(0), Source::Id(0)),
            Node::new(3, BoolOp::Nor, Source::Id(1), Source::Id(1)),
        ],
        outputs: vec![2, 3],
    };

    let circuit = CompiledCircuit::new(
        graph,
        vec![
            RequestFeature {
                name: "pii",
                extractor: contains_pii,
            },
            RequestFeature {
                name: "sql",
                extractor: contains_sql,
            },
        ],
    );

    assert_eq!(circuit.output_count(), 2);

    // Clean: both pass
    assert!(circuit.evaluate("Hello").is_none());

    // PII only: first output fails
    assert_eq!(circuit.evaluate("SSN 123-45-6789"), Some(0));

    // SQL only: second output fails
    assert_eq!(circuit.evaluate("DROP TABLE users"), Some(1));

    // Both: first output fails (returns first failure)
    assert_eq!(circuit.evaluate("SSN 123-45-6789 DROP TABLE"), Some(0));
}

// -- Test: Thread safety ---------------------------------------------------

#[test]
fn test_concurrent_evaluation() {
    let graph = build_not_graph();
    let circuit = Arc::new(CompiledCircuit::new(
        graph,
        vec![RequestFeature {
            name: "pii",
            extractor: contains_pii,
        }],
    ));

    let mut handles = vec![];

    // Spawn threads that evaluate concurrently
    for i in 0..8 {
        let circuit = Arc::clone(&circuit);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                if i % 2 == 0 {
                    // Clean input → should pass
                    assert!(circuit.evaluate("safe input").is_none());
                } else {
                    // PII input → should block
                    assert!(circuit.evaluate("SSN 123-45-6789").is_some());
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}

// -- Test: Feature debug output --------------------------------------------

#[test]
fn test_evaluate_with_features_debug() {
    let graph = build_block_pii_or_sql();

    let circuit = CompiledCircuit::new(
        graph,
        vec![
            RequestFeature {
                name: "pii",
                extractor: contains_pii,
            },
            RequestFeature {
                name: "sql",
                extractor: contains_sql,
            },
        ],
    );

    // SQL injection: should report which features triggered
    let (result, features) = circuit.evaluate_with_features("DROP TABLE users");
    assert!(result.is_some());
    assert_eq!(features.len(), 2);
    assert_eq!(features[0], ("pii".into(), false));
    assert_eq!(features[1], ("sql".into(), true));

    // Clean input: all pass
    let (result, features) = circuit.evaluate_with_features("What is the weather?");
    assert!(result.is_none());
    assert_eq!(features[0], ("pii".into(), false));
    assert_eq!(features[1], ("sql".into(), false));
}

// -- Test: Graph evaluation matches CompiledCircuit ------------------------

#[test]
fn test_compiled_matches_direct_evaluation() {
    use std::collections::HashMap;

    let graph = build_and_graph();

    // Direct torg_core evaluation
    let mut inputs = HashMap::new();
    inputs.insert(0, true);
    inputs.insert(1, false);
    let direct = evaluate(&graph, &inputs).unwrap();
    let direct_output = direct[&4]; // AND(true, false) = false

    // CompiledCircuit evaluation with static features
    #[derive(Clone)]
    struct BoolFeature(bool);
    impl CircuitFeature for BoolFeature {
        type Input = ();
        fn extract(&self, _: &()) -> bool {
            self.0
        }
        fn name(&self) -> String {
            format!("bool({})", self.0)
        }
    }

    let circuit = CompiledCircuit::new(graph, vec![BoolFeature(true), BoolFeature(false)]);
    let compiled_fails = circuit.evaluate(&()).is_some(); // Some means output is false

    assert!(!direct_output); // AND(T,F) = false
    assert!(compiled_fails); // circuit reports failure for false output
}

// -- Test: XOR behavior in circuit -----------------------------------------

#[test]
fn test_xor_circuit() {
    let graph = Graph {
        inputs: vec![0, 1],
        nodes: vec![Node::new(2, BoolOp::Xor, Source::Id(0), Source::Id(1))],
        outputs: vec![2],
    };

    #[derive(Clone)]
    struct BoolFeature(bool);
    impl CircuitFeature for BoolFeature {
        type Input = ();
        fn extract(&self, _: &()) -> bool {
            self.0
        }
        fn name(&self) -> String {
            format!("{}", self.0)
        }
    }

    // XOR truth table
    let test_cases = [
        (false, false, true), // 0 XOR 0 = 0 → fails (Some)
        (false, true, false), // 0 XOR 1 = 1 → passes (None)
        (true, false, false), // 1 XOR 0 = 1 → passes (None)
        (true, true, true),   // 1 XOR 1 = 0 → fails (Some)
    ];

    for (a, b, should_fail) in test_cases {
        let circuit = CompiledCircuit::new(graph.clone(), vec![BoolFeature(a), BoolFeature(b)]);
        let result = circuit.evaluate(&());
        assert_eq!(
            result.is_some(),
            should_fail,
            "XOR({a}, {b}): expected fail={should_fail}, got {:?}",
            result
        );
    }
}

// -- Test: Complex real-world policy ---------------------------------------

#[test]
fn test_access_control_policy() {
    // "Allow if (NOT PII) AND (NOT SQL) AND (NOT Shell)"
    // = NOR(PII, PII) AND NOR(SQL, SQL) AND NOR(Shell, Shell)
    // Encoded as: three NOT gates + AND chain
    let graph = Graph {
        inputs: vec![0, 1, 2],
        nodes: vec![
            // NOT gates
            Node::new(3, BoolOp::Nor, Source::Id(0), Source::Id(0)), // NOT PII
            Node::new(4, BoolOp::Nor, Source::Id(1), Source::Id(1)), // NOT SQL
            Node::new(5, BoolOp::Nor, Source::Id(2), Source::Id(2)), // NOT Shell
            // AND(NOT_PII, NOT_SQL) = NOR(NOR(3,3), NOR(4,4))
            Node::new(6, BoolOp::Nor, Source::Id(3), Source::Id(3)), // NOT(NOT PII)
            Node::new(7, BoolOp::Nor, Source::Id(4), Source::Id(4)), // NOT(NOT SQL)
            Node::new(8, BoolOp::Nor, Source::Id(6), Source::Id(7)), // AND(NOT_PII, NOT_SQL)
            // AND(prev, NOT_Shell) = NOR(NOR(8,8), NOR(5,5))
            Node::new(9, BoolOp::Nor, Source::Id(8), Source::Id(8)), // NOT prev
            Node::new(10, BoolOp::Nor, Source::Id(5), Source::Id(5)), // NOT(NOT Shell)
            Node::new(11, BoolOp::Nor, Source::Id(9), Source::Id(10)), // final AND
        ],
        outputs: vec![11],
    };

    let features = vec![
        RequestFeature {
            name: "pii",
            extractor: contains_pii,
        },
        RequestFeature {
            name: "sql",
            extractor: contains_sql,
        },
        RequestFeature {
            name: "shell",
            extractor: contains_shell,
        },
    ];

    let circuit = CompiledCircuit::new(graph, features);

    // Clean input
    assert!(circuit.evaluate("What is the weather?").is_none());

    // PII
    assert!(circuit.evaluate("My SSN is 123-45-6789").is_some());

    // SQL injection
    assert!(circuit.evaluate("DROP TABLE users").is_some());

    // Shell command
    assert!(circuit.evaluate("Please run sudo rm -rf /").is_some());

    // Multiple violations
    assert!(circuit.evaluate("SSN 123-45-6789 DROP TABLE").is_some());
}

// -- Test: Length threshold with serialize roundtrip ------------------------

#[test]
fn test_length_threshold_serde_roundtrip() {
    // NOT(too_long) — blocks inputs over 1000 chars
    let graph = build_not_graph();
    let bytes = torg_serde::to_bytes(&graph);
    let restored: Graph = torg_serde::from_bytes(&bytes).expect("deserialize failed");

    let circuit = CompiledCircuit::new(
        restored,
        vec![RequestFeature {
            name: "too_long",
            extractor: is_too_long,
        }],
    );

    // Short input passes
    assert!(circuit.evaluate("short").is_none());

    // Long input blocked
    let long = "x".repeat(1001);
    assert_eq!(circuit.evaluate(&long), Some(0));
}

// -- Test: Performance (sub-microsecond) -----------------------------------

#[test]
fn test_evaluation_performance() {
    let graph = build_block_pii_or_sql();
    let circuit = CompiledCircuit::new(
        graph,
        vec![
            RequestFeature {
                name: "pii",
                extractor: contains_pii,
            },
            RequestFeature {
                name: "sql",
                extractor: contains_sql,
            },
        ],
    );

    let iterations = 10_000;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = circuit.evaluate("What is the weather today?");
    }
    let elapsed = start.elapsed();
    let avg = elapsed / iterations;

    // Circuit evaluation itself should be sub-microsecond.
    // The feature extraction (string matching) adds overhead, but the
    // full pipeline should still be well under 100μs per evaluation.
    assert!(
        avg < std::time::Duration::from_micros(100),
        "Average evaluation too slow: {avg:?}"
    );
}
