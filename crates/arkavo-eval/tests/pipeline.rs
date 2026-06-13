use arkavo_eval::baseline::MemBaselineStore;
use arkavo_eval::contract::*;
use arkavo_eval::gate::Preconditions;
use arkavo_eval::operator::FakeOperator;
use arkavo_eval::run_eval;
use arkavo_eval::status::TypedStatus;
use arkavo_eval::verdict::{Embedder, VerdictError};
use async_trait::async_trait;
use std::collections::HashMap;

struct FakeEmbedder;
#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        let mut v = vec![0.0f32; 27];
        for c in text.to_lowercase().chars() {
            if c.is_ascii_lowercase() {
                v[(c as u8 - b'a') as usize] += 1.0;
            } else {
                v[26] += 1.0;
            }
        }
        Ok(v)
    }
}

fn contract(commit: &str) -> EvalContract {
    EvalContract {
        contract_id: "id".into(),
        task_kind: "model_eval".into(),
        model: ModelSpec {
            name: "gemma-4-12b".into(),
            quant: "Q4_K_M".into(),
            weight_digest: "b3:0".into(),
        },
        baseline: BaselineRef {
            kind: "reference_outputs".into(),
            commit: Some(commit.into()),
            digest: None,
        },
        prompts: vec![EvalPrompt {
            id: "capital".into(),
            messages: vec![PromptMessage {
                role: "user".into(),
                content: "Capital of France?".into(),
            }],
            tools: None,
        }],
        acceptance: Acceptance {
            min_similarity: 0.87,
            min_tok_s_ratio: 0.95,
        },
        execution: ExecutionProfile {
            seed: 0,
            temperature: 0.0,
            threads: None,
            ctx: None,
            max_tokens: 32,
        },
        preconditions: vec!["weights_present".into(), "baseline_present".into()],
        policy_circuit: "torg:eval-preflight-v1".into(),
        on_precondition_unmet: "refuse".into(),
    }
}

fn op(answer: &str) -> FakeOperator {
    let mut a = HashMap::new();
    a.insert("capital".to_string(), answer.to_string());
    FakeOperator {
        answers: a,
        tok_s: 100.0,
    }
}

fn pre(baseline_present: bool) -> Preconditions {
    Preconditions {
        weights_present: true,
        weights_attested: true,
        provenance_valid: true,
        baseline_present,
    }
}

#[tokio::test]
async fn refused_when_precondition_unmet() {
    let store = MemBaselineStore::new();
    let outcome = run_eval(
        &contract("c1"),
        &pre(false),
        &op("Paris"),
        &store,
        &FakeEmbedder,
        false,
    )
    .await;
    assert!(matches!(outcome.status, TypedStatus::Refused { .. }));
}

#[tokio::test]
async fn bootstraps_on_main_then_passes_on_pr() {
    let store = MemBaselineStore::new();
    // First, on main with no baseline → bootstrap + publish.
    let boot = run_eval(
        &contract("c1"),
        &pre(true),
        &op("Paris"),
        &store,
        &FakeEmbedder,
        true,
    )
    .await;
    assert_eq!(boot.status, TypedStatus::BaselineBootstrapped);
    assert!(boot.published.is_some());
    // Then a PR run with the same answer → passes.
    let pass = run_eval(
        &contract("c1"),
        &pre(true),
        &op("Paris"),
        &store,
        &FakeEmbedder,
        false,
    )
    .await;
    assert_eq!(pass.status, TypedStatus::Passed);
    assert!(pass.published.is_none());
}

#[tokio::test]
async fn regression_when_output_diverges() {
    let store = MemBaselineStore::new();
    run_eval(
        &contract("c1"),
        &pre(true),
        &op("Paris"),
        &store,
        &FakeEmbedder,
        true,
    )
    .await;
    let reg = run_eval(
        &contract("c1"),
        &pre(true),
        &op("zzzzzz"),
        &store,
        &FakeEmbedder,
        false,
    )
    .await;
    assert!(matches!(reg.status, TypedStatus::RegressionFailed { .. }));
}
