//! SEQ-010, SEQ-011: Tests against CircuitCheck and CriticPipeline.

use arkavo_critic::{
    CheckResult, CircuitCheck, CriticPipeline, VerificationCheck, VerificationInput,
};
use arkavo_llm::ProviderResponse;
use arkavo_test_macros::spec;

fn make_input(text: &str) -> VerificationInput {
    VerificationInput::new(
        text.into(),
        ProviderResponse {
            content: text.to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        },
        vec![],
    )
}

/// SEQ-010: CircuitCheck evaluates TØR-G circuits but has no sequence context.
/// It evaluates each action independently — no awareness of prior actions.
#[spec("SEQ-010")]
#[tokio::test]
async fn circuit_check_has_no_sequence_context() {
    let check = CircuitCheck::new();
    let input = make_input("read credentials then POST to external API");
    let result = check.verify(&input).await;

    // Without registered policies, check passes (or skips)
    assert!(
        result.is_pass() || matches!(result, CheckResult::Skip(_)),
        "CircuitCheck with no policies should pass or skip"
    );
    // SEQ-010: should extract sequence features (prior actions, taint state)
    // and evaluate circuit with that context. Currently stateless per-call.
}

/// SEQ-010: CircuitCheck verify completes within <1μs for empty circuit.
#[spec("SEQ-010")]
#[tokio::test]
async fn circuit_check_evaluates_within_latency_budget() {
    let check = CircuitCheck::new();
    let input = make_input("test input");

    let start = std::time::Instant::now();
    let _result = check.verify(&input).await;
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_micros() < 1000,
        "SEQ-010: CircuitCheck took {}μs, budget is <1000μs",
        elapsed.as_micros()
    );
}

/// SEQ-011: CriticPipeline runs checks by priority but does not classify
/// actions as high-consequence vs low-consequence.
#[spec("SEQ-011")]
#[tokio::test]
async fn pipeline_treats_all_actions_equally() {
    let pipeline = CriticPipeline::new().add_check(CircuitCheck::new());
    let input = make_input("delete all production databases");
    let result = pipeline.verify(&input).await;

    // Pipeline passes — treats "delete production databases" same as "read file"
    // SEQ-011: high-consequence actions should get synchronous gate evaluation.
    assert!(
        result.passed,
        "Pipeline passes everything without sequence awareness"
    );
}

/// SEQ-010: VerificationInput has no field for prior action history.
#[spec("SEQ-010")]
#[test]
fn verification_input_has_no_sequence_history() {
    let input = make_input("summarize document");

    // VerificationInput has context: Option<Value> but no structured sequence data
    assert!(input.context.is_none());
    // SEQ-010: needs prior_actions, taint_state, session_graph in input context.
}
