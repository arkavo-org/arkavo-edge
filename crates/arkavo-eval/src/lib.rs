//! Local-model evaluation pipeline: resolves an eval contract, gates on
//! preconditions, runs the model, and produces a typed regression verdict.

pub mod baseline;
pub mod contract;
pub mod digest;
pub mod gate;
pub mod operator;
#[cfg(feature = "llama-cpp")]
pub mod operator_llama;
pub mod plan;
pub mod status;
pub mod verdict;

use baseline::{BaselinePointer, BaselineStore};
use contract::EvalContract;
use gate::{evaluate_gate, Preconditions};
use operator::Operator;
use status::TypedStatus;
use verdict::{assess, Baseline, BaselineOutput, Embedder};

/// Outcome of an eval run: the terminal status plus any baseline published
/// (only on `main` runs).
#[derive(Debug)]
pub struct RunOutcome {
    pub status: TypedStatus,
    pub published: Option<BaselinePointer>,
}

/// Run the full pipeline. `is_main` is true when this run is on the default
/// branch after merge, in which case a passing/bootstrap run records the new
/// baseline.
pub async fn run_eval<O, B, E>(
    contract: &EvalContract,
    pre: &Preconditions,
    operator: &O,
    baselines: &B,
    embed: &E,
    is_main: bool,
) -> RunOutcome
where
    O: Operator,
    B: BaselineStore,
    E: Embedder,
{
    // 1. Pre-flight gate.
    if let Some(refused) = evaluate_gate(pre, &contract.preconditions).into_status_if_denied() {
        return RunOutcome {
            status: refused,
            published: None,
        };
    }

    // 2. Plan + run the model.
    let evplan = plan::plan(contract);
    let run = match operator.run(&evplan).await {
        Ok(r) => r,
        Err(e) => {
            return RunOutcome {
                status: TypedStatus::InfraError {
                    stage: format!("operator: {e}"),
                },
                published: None,
            };
        }
    };

    let commit = evplan.baseline_commit.clone().unwrap_or_default();
    let model = contract.model.name.clone();

    // 3. Fetch the baseline.
    let baseline = match baselines.fetch(&commit, &model).await {
        Ok(b) => b,
        Err(e) => {
            return RunOutcome {
                status: TypedStatus::InfraError {
                    stage: format!("historian: {e}"),
                },
                published: None,
            };
        }
    };

    match baseline {
        // 4a. No baseline yet → bootstrap. On main, publish; on PR, neutral.
        None => {
            let new_baseline = Baseline {
                outputs: run
                    .outputs
                    .iter()
                    .map(|o| BaselineOutput {
                        id: o.id.clone(),
                        text: o.text.clone(),
                    })
                    .collect(),
                tok_s: mean_tok_s(&run.outputs),
            };
            let published = if is_main {
                baselines.publish(&commit, &model, &new_baseline).await.ok()
            } else {
                None
            };
            RunOutcome {
                status: TypedStatus::BaselineBootstrapped,
                published,
            }
        }
        // 4b. Baseline exists → assess.
        Some(base) => {
            let status = match assess(
                embed,
                &run.outputs,
                &base,
                contract.acceptance.min_similarity,
                contract.acceptance.min_tok_s_ratio,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => TypedStatus::InfraError {
                    stage: format!("verdict: {e}"),
                },
            };
            // On a passing main run, record the new baseline (promotion on merge).
            let published = if is_main && status == TypedStatus::Passed {
                let new_baseline = Baseline {
                    outputs: run
                        .outputs
                        .iter()
                        .map(|o| BaselineOutput {
                            id: o.id.clone(),
                            text: o.text.clone(),
                        })
                        .collect(),
                    tok_s: mean_tok_s(&run.outputs),
                };
                baselines.publish(&commit, &model, &new_baseline).await.ok()
            } else {
                None
            };
            RunOutcome { status, published }
        }
    }
}

fn mean_tok_s(outputs: &[operator::PromptOutput]) -> f64 {
    if outputs.is_empty() {
        return 0.0;
    }
    outputs.iter().map(|o| o.tok_s).sum::<f64>() / outputs.len() as f64
}
