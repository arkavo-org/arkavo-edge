//! Integration tests for PreflightModerator in Router flow
//!
//! Tests the Safety Validation Suite from docs/llm-integration-testing-requirements.md Section 3.3.3

use arkavo_router::Error;
use arkavo_router::preflight::{ModerationResult, PolicyId, PreflightFeature, PreflightModerator};
use arkavo_test_macros::spec;
use torg_core::{BoolOp, Graph, Node, Source};

/// Build a NOT circuit: output = NOT(input0)
/// NOR(A, A) = NOT(A)
fn build_not_circuit() -> Graph {
    Graph {
        inputs: vec![0],
        nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))],
        outputs: vec![1],
    }
}

#[spec("ROUTER-004")]
#[test]
fn test_error_type_conversion() {
    let moderator = PreflightModerator::new();
    moderator.register_graph(
        PolicyId::new("block_pii"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsPII],
    );

    if let ModerationResult::Block {
        policy_id, reason, ..
    } = moderator.check("My SSN is 123-45-6789")
    {
        let err = Error::ModerationBlocked { policy_id, reason };
        assert!(err.to_string().contains("block_pii"));
    } else {
        panic!("Expected Block result");
    }
}

/// Safety Validation Suite from requirements doc Section 3.3.3
#[spec("ROUTER-004")]
#[test]
fn test_safety_validation_suite() {
    let moderator = PreflightModerator::new();

    moderator.register_graph(
        PolicyId::new("block_pii"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsPII],
    );
    moderator.register_graph(
        PolicyId::new("block_injection"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsSQLKeywords],
    );
    moderator.register_graph(
        PolicyId::new("block_shell"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsShellCommands],
    );

    // Test cases from requirements table
    // | Test Case | Input | Expected Result |
    // | PII detection blocks SSN | "My SSN is 123-45-6789" | Block(policy: "block_pii") |
    assert!(moderator.check("My SSN is 123-45-6789").is_blocked());

    // | PII detection blocks credit card | "Card: 4111-1111-1111-1111" | Block(policy: "block_pii") |
    assert!(moderator.check("Card: 4111-1111-1111-1111").is_blocked());

    // | Clean input passes | "What is the weather?" | Allow |
    assert!(moderator.check("What is the weather?").is_allowed());

    // | SQL injection blocked | "DROP TABLE users;" | Block(policy: "block_injection") |
    assert!(moderator.check("DROP TABLE users;").is_blocked());

    // | Shell command blocked | "Run sudo rm -rf /" | Block(policy: "block_shell") |
    assert!(moderator.check("Run sudo rm -rf /").is_blocked());

    // | Empty input | "" | Allow |
    assert!(moderator.check("").is_allowed());
}

/// Test that no circuits registered means all traffic allowed
#[spec("ROUTER-004")]
#[test]
fn test_no_circuits_allows_all() {
    let moderator = PreflightModerator::new();

    // With no circuits, everything should pass
    assert!(moderator.check("My SSN is 123-45-6789").is_allowed());
    assert!(moderator.check("DROP TABLE users;").is_allowed());
    assert!(moderator.check("Run sudo rm -rf /").is_allowed());
}

/// Test long input with length threshold policy
#[spec("ROUTER-004")]
#[test]
fn test_long_input_blocked() {
    let moderator = PreflightModerator::new();
    moderator.register_graph(
        PolicyId::new("length_limit"),
        build_not_circuit(),
        vec![PreflightFeature::InputLengthExceedsThreshold(100_000)],
    );

    // 100k+ characters should be blocked
    let long_input = "x".repeat(100_001);
    let result = moderator.check(&long_input);
    assert!(result.is_blocked());
    if let ModerationResult::Block { policy_id, .. } = result {
        assert_eq!(policy_id, "length_limit");
    }
}
