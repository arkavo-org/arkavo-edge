//! Operator role: runs the model over the plan's prompts and captures the
//! output text and tokens/sec per prompt. The real llama.cpp implementation
//! lands in Part 2 behind the `llama-cpp` feature; this module defines the
//! trait and a fake used by tests and the one-shot CLI demo.

use crate::plan::EvalPlan;
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OperatorError {
    #[error("model load failed: {0}")]
    Load(String),
    #[error("generation failed: {0}")]
    Generate(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptOutput {
    pub id: String,
    pub text: String,
    pub tok_s: f64,
}

#[derive(Debug, Clone)]
pub struct RunOutput {
    pub outputs: Vec<PromptOutput>,
}

#[async_trait]
pub trait Operator: Send + Sync {
    async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError>;
}

/// Compute a real, non-zero tokens/sec figure for the throughput check.
/// Prefers the engine's own generation timing (`generation_ms` + `n_eval`);
/// some llama.cpp builds leave that zeroed, so it falls back to wall-clock
/// elapsed over the known (or estimated) token count. A grounded measurement
/// beats a silent `0.0`, which would neutralize the regression gate.
///
/// Gated to the configs that consume it: the `llama-cpp` operator and the unit
/// tests. The arithmetic still lives here (not in the feature-gated operator) so
/// the regression test compiles without building llama.cpp.
#[cfg(any(feature = "llama-cpp", test))]
pub(crate) fn measured_tok_s(gen_ms: f64, n_eval: u32, elapsed_s: f64, text: &str) -> f64 {
    if gen_ms > 0.0 && n_eval > 0 {
        return n_eval as f64 / (gen_ms / 1000.0);
    }
    if elapsed_s <= 0.0 {
        return 0.0;
    }
    let tokens = if n_eval > 0 {
        n_eval
    } else {
        estimate_tokens(text)
    };
    tokens as f64 / elapsed_s
}

/// Rough token estimate when the engine reports no count: the larger of
/// word-count×1.3 and chars/4, matching typical sub-word tokenization. Empty
/// output yields 0 so a model that generated nothing reports no throughput.
#[cfg(any(feature = "llama-cpp", test))]
fn estimate_tokens(text: &str) -> u32 {
    if text.trim().is_empty() {
        return 0;
    }
    let by_words = text.split_whitespace().count() as f64 * 1.3;
    let by_chars = text.chars().count() as f64 / 4.0;
    by_words.max(by_chars).max(1.0).round() as u32
}

/// Returns a fixed answer per prompt id. Used by tests and the CLI demo.
pub struct FakeOperator {
    pub answers: std::collections::HashMap<String, String>,
    pub tok_s: f64,
}

#[async_trait]
impl Operator for FakeOperator {
    async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError> {
        let outputs = plan
            .prompts
            .iter()
            .map(|p| PromptOutput {
                id: p.id.clone(),
                text: self.answers.get(&p.id).cloned().unwrap_or_default(),
                tok_s: self.tok_s,
            })
            .collect();
        Ok(RunOutput { outputs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{EvalPrompt, ExecutionProfile, ModelSpec, PromptMessage};
    use std::collections::HashMap;

    #[tokio::test]
    async fn fake_operator_answers_by_id() {
        let plan = EvalPlan {
            model: ModelSpec {
                name: "m".into(),
                quant: "q".into(),
                weight_digest: "b3:0".into(),
            },
            prompts: vec![EvalPrompt {
                id: "p1".into(),
                messages: vec![PromptMessage {
                    role: "user".into(),
                    content: "hi".into(),
                }],
                tools: None,
            }],
            exec: ExecutionProfile {
                seed: 0,
                temperature: 0.0,
                threads: None,
                ctx: None,
                max_tokens: 8,
            },
            baseline_commit: None,
        };
        let mut answers = HashMap::new();
        answers.insert("p1".to_string(), "hello".to_string());
        let op = FakeOperator {
            answers,
            tok_s: 42.0,
        };
        let out = op.run(&plan).await.unwrap();
        assert_eq!(out.outputs.len(), 1);
        assert_eq!(out.outputs[0].text, "hello");
        assert_eq!(out.outputs[0].tok_s, 42.0);
    }

    #[test]
    fn tok_s_prefers_engine_generation_rate() {
        // 50 tokens in 100 ms = 500 tok/s; wall-clock is ignored when the engine reports.
        let r = measured_tok_s(100.0, 50, 5.0, "anything");
        assert!((r - 500.0).abs() < 1e-6, "got {r}");
    }

    #[test]
    fn tok_s_falls_back_to_wallclock_with_known_token_count() {
        // Engine left generation_ms at 0 but n_eval is known: use wall-clock.
        // 50 tokens / 2 s = 25 tok/s.
        let r = measured_tok_s(0.0, 50, 2.0, "anything");
        assert!((r - 25.0).abs() < 1e-6, "got {r}");
    }

    #[test]
    fn tok_s_estimates_tokens_when_engine_reports_nothing() {
        // No timing at all: estimate token count from the text over wall-clock.
        // Must be a real, non-zero throughput.
        let r = measured_tok_s(0.0, 0, 2.0, "The capital of Australia is Canberra.");
        assert!(r > 0.0, "expected non-zero throughput, got {r}");
    }

    #[test]
    fn tok_s_is_zero_when_unmeasurable() {
        // No engine timing and no elapsed time -> cannot measure -> 0.0 (honest).
        assert_eq!(measured_tok_s(0.0, 0, 0.0, "text"), 0.0);
        // Empty output -> nothing was generated -> 0.0.
        assert_eq!(measured_tok_s(0.0, 0, 1.0, "   "), 0.0);
    }
}
