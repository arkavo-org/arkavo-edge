//! Integration tests for CircuitCheck post-LLM validation
//!
//! Tests CircuitCheck integration per docs/llm-integration-testing-requirements.md Section 3.3.2

use arkavo_critic::{CircuitCheck, FeatureId, PolicyId, VerificationCheck, VerificationInput};
use arkavo_llm::ProviderResponse;
use arkavo_llm::tool_parser::ParsedToolCall;
use torg_core::{BoolOp, Graph, Node, Source};
use torg_serde::to_bytes;

/// Build a NOT circuit: output = NOT(input0)
/// NOR(A, A) = NOT(A)
fn build_not_circuit() -> Graph {
    Graph {
        inputs: vec![0],
        nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))],
        outputs: vec![1],
    }
}

fn make_response(content: &str, tool_calls: Vec<ParsedToolCall>) -> ProviderResponse {
    ProviderResponse {
        content: content.to_string(),
        reasoning_content: None,
        tool_calls,
        finish_reason: None,
    }
}

/// Test that CircuitCheck blocks LLM responses with tool calls when policy forbids them
#[tokio::test]
async fn test_circuit_blocks_response_with_tool_calls() {
    let check = CircuitCheck::new();
    let graph = build_not_circuit();
    let bytes = to_bytes(&graph);

    // Policy: Block if HasToolCalls is true (NOT circuit inverts)
    check
        .register(
            PolicyId::new("block_tool_calls"),
            &bytes,
            vec![FeatureId::HasToolCalls],
        )
        .unwrap();

    // LLM response with a tool call (simulating hallucinated/unwanted tool)
    let bad_response = make_response(
        "I'll use the imaginary_api tool",
        vec![ParsedToolCall {
            tool_name: "imaginary_api".into(),
            arguments: serde_json::json!({"action": "do_something"}),
            call_id: None,
        }],
    );

    let input = VerificationInput::new("test".into(), bad_response, vec![]);
    let result = check.verify(&input).await;
    assert!(result.is_fail());
}

/// Test that CircuitCheck allows clean responses with no tool calls
#[tokio::test]
async fn test_circuit_allows_clean_response() {
    let check = CircuitCheck::new();
    let graph = build_not_circuit();
    let bytes = to_bytes(&graph);

    check
        .register(
            PolicyId::new("block_tool_calls"),
            &bytes,
            vec![FeatureId::HasToolCalls],
        )
        .unwrap();

    // Clean LLM response with no tool calls
    let good_response = make_response("The weather is sunny today.", vec![]);

    let input = VerificationInput::new("test".into(), good_response, vec![]);
    let result = check.verify(&input).await;
    assert!(result.is_pass());
}

/// Test that CircuitCheck with no circuits skips verification
#[tokio::test]
async fn test_circuit_skips_when_empty() {
    let check = CircuitCheck::new();

    // No circuits registered - should skip
    let response = make_response("Any response", vec![]);
    let input = VerificationInput::new("test".into(), response, vec![]);
    let result = check.verify(&input).await;

    // When no circuits registered, check should skip (pass)
    // Skip variant indicates not applicable
    assert!(matches!(
        result,
        arkavo_critic::CheckResult::Pass | arkavo_critic::CheckResult::Skip(_)
    ));
}

/// Test policy that blocks responses containing code
#[tokio::test]
async fn test_circuit_blocks_code_in_response() {
    let check = CircuitCheck::new();
    let graph = build_not_circuit();
    let bytes = to_bytes(&graph);

    // Policy: Block if ResponseHasCode is true
    check
        .register(
            PolicyId::new("block_code"),
            &bytes,
            vec![FeatureId::ResponseHasCode],
        )
        .unwrap();

    // Response with code block should be blocked
    let code_response = make_response("Here's the code:\n```rust\nfn main() {}\n```", vec![]);
    let input = VerificationInput::new("test".into(), code_response, vec![]);
    let result = check.verify(&input).await;
    assert!(result.is_fail());

    // Response without code should pass
    let clean_response = make_response("The answer is 42.", vec![]);
    let input = VerificationInput::new("test".into(), clean_response, vec![]);
    let result = check.verify(&input).await;
    assert!(result.is_pass());
}
