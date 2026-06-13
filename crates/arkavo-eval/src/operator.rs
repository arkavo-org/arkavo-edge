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
}
