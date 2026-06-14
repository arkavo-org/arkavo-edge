//! Real Operator: loads a GGUF via arkavo-llm's llama.cpp provider and runs the
//! prompt-set under the contract's execution profile, capturing output text and
//! tokens/sec. The fake counterpart lives in `operator.rs`.

use crate::operator::{Operator, OperatorError, PromptOutput, RunOutput};
use crate::plan::EvalPlan;
use arkavo_llm::{LlamaCppProvider, Message, Provider, SamplingConfig};
use async_trait::async_trait;

pub struct LlamaOperator {
    pub model_name: String,
    pub model_path: String,
}

impl LlamaOperator {
    pub fn new(model_name: impl Into<String>, model_path: impl Into<String>) -> Self {
        Self {
            model_name: model_name.into(),
            model_path: model_path.into(),
        }
    }

    fn message(role: &str, content: &str) -> Message {
        match role {
            "system" => Message::system(content),
            "assistant" => Message::assistant(content),
            _ => Message::user(content),
        }
    }
}

#[async_trait]
impl Operator for LlamaOperator {
    async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError> {
        let config = SamplingConfig {
            temperature: plan.exec.temperature,
            seed: plan.exec.seed,
            max_tokens: plan.exec.max_tokens,
            ..Default::default()
        };
        let provider = LlamaCppProvider::new_with_config(
            self.model_name.clone(),
            self.model_path.clone(),
            None,
            config,
        )
        .map_err(|e| OperatorError::Load(e.to_string()))?;

        let mut outputs = Vec::with_capacity(plan.prompts.len());
        for prompt in &plan.prompts {
            let messages: Vec<Message> = prompt
                .messages
                .iter()
                .map(|m| Self::message(&m.role, &m.content))
                .collect();
            let started = std::time::Instant::now();
            let resp = provider
                .complete_with_tools(
                    messages,
                    prompt.tools.clone(),
                    Some(plan.exec.max_tokens as usize),
                )
                .await
                .map_err(|e| OperatorError::Generate(e.to_string()))?;
            let elapsed_s = started.elapsed().as_secs_f64();

            // Fold any tool calls into the compared text so tool-selection
            // regressions are visible to the similarity check.
            let mut text = resp.content.clone();
            for tc in &resp.tool_calls {
                text.push_str(&format!("\n[tool:{} {}]", tc.tool_name, tc.arguments));
            }

            let (gen_ms, n_eval) = resp
                .inference_timing
                .as_ref()
                .map(|t| (t.generation_ms, t.n_eval))
                .unwrap_or((0.0, 0));
            let tok_s = crate::operator::measured_tok_s(gen_ms, n_eval, elapsed_s, &text);

            outputs.push(PromptOutput {
                id: prompt.id.clone(),
                text,
                tok_s,
            });
        }
        Ok(RunOutput { outputs })
    }
}
