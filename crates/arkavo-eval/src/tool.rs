//! `run_eval` MCP tool: runs the local-model eval suite on a model and returns
//! a pass/regression verdict vs the recorded baseline. Registered into the
//! agent loop's tool registry so the conductor can call it.

use crate::baseline::BaselineStore;
use crate::contract::{EvalPrompt, ExecutionProfile, ModelSpec, PromptMessage};
use crate::operator::Operator;
use crate::operator_llama::LlamaOperator;
use crate::plan::EvalPlan;
use crate::verdict::{assess, Baseline, BaselineOutput, Embedder};
use arkavo_mcp_tools::server::Tool;
use arkavo_mcp_tools::{ToolError, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Resolves a model name to a local GGUF path.
pub type ModelResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Shared state for the eval tools.
pub struct EvalState {
    pub embedder: Arc<dyn Embedder>,
    pub baselines: Arc<dyn BaselineStore>,
    /// Default capability prompt-set used when the caller doesn't supply one.
    pub prompts: Vec<EvalPrompt>,
    /// Resolve a model name to a local GGUF path (e.g. from the HF cache).
    pub resolve_model: ModelResolver,
}

impl EvalState {
    /// The default capability prompt-set.
    pub fn default_prompts() -> Vec<EvalPrompt> {
        let user = |id: &str, content: &str| EvalPrompt {
            id: id.into(),
            messages: vec![PromptMessage {
                role: "user".into(),
                content: content.into(),
            }],
            tools: None,
        };
        vec![
            user("capital_au", "What is the capital of Australia? Answer with one word."),
            user("arithmetic", "What is 17 multiplied by 23? Answer with just the number."),
            user("reverse", "Reverse the letters of the word 'algorithm'. Answer with only the reversed string, nothing else."),
            user("symbol", "What is the chemical symbol for gold? Answer with just the symbol."),
            user("primes", "List the first five prime numbers, comma-separated, nothing else."),
        ]
    }
}

pub struct RunEvalTool {
    schema: ToolSchema,
    state: Arc<EvalState>,
}

impl RunEvalTool {
    pub fn new(state: Arc<EvalState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "run_eval".to_string(),
                aliases: None,
                description: "Run the local-model evaluation suite on a model and return a pass/regression verdict versus the recorded baseline. Use to gate PRs that change model behavior. Args: model (name), baseline_ref (label the baseline is keyed under, e.g. a git ref; default 'main'), update_baseline (record this run as the new baseline).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "model": { "type": "string", "description": "Model name to evaluate (resolved to a local GGUF)." },
                        "baseline_ref": { "type": "string", "description": "Key the baseline is stored under. Default 'main'." },
                        "update_baseline": { "type": "boolean", "description": "If true, record this run as the new baseline. Default false." }
                    },
                    "required": ["model"]
                }),
            },
            state,
        }
    }
}

#[async_trait]
impl Tool for RunEvalTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Execution("run_eval: missing 'model'".into()))?;
        let baseline_ref = args
            .get("baseline_ref")
            .and_then(|v| v.as_str())
            .unwrap_or("main");
        let update_baseline = args
            .get("update_baseline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = (self.state.resolve_model)(model).ok_or_else(|| {
            ToolError::Execution(format!(
                "run_eval: model '{model}' not resident on this swarm member"
            ))
        })?;

        let plan = EvalPlan {
            model: ModelSpec {
                name: model.to_string(),
                quant: "Q4_K_M".into(),
                weight_digest: "b3:agent".into(),
            },
            prompts: self.state.prompts.clone(),
            exec: ExecutionProfile {
                seed: 0,
                temperature: 0.0,
                threads: None,
                ctx: None,
                max_tokens: 48,
            },
            baseline_commit: Some(baseline_ref.to_string()),
        };

        let run = LlamaOperator::new(model.to_string(), path)
            .run(&plan)
            .await
            .map_err(|e| ToolError::Execution(format!("run_eval: {e}")))?;

        let existing = self
            .state
            .baselines
            .fetch(baseline_ref, model)
            .await
            .map_err(|e| ToolError::Execution(format!("run_eval baseline fetch: {e}")))?;

        let mean_tok_s = if run.outputs.is_empty() {
            0.0
        } else {
            run.outputs.iter().map(|o| o.tok_s).sum::<f64>() / run.outputs.len() as f64
        };
        let outputs_json: Vec<Value> = run
            .outputs
            .iter()
            .map(|o| json!({ "id": o.id, "text": o.text, "tok_s": o.tok_s }))
            .collect();

        let (status, recorded) = match (&existing, update_baseline) {
            (None, _) | (Some(_), true) => {
                let new_baseline = Baseline {
                    outputs: run
                        .outputs
                        .iter()
                        .map(|o| BaselineOutput {
                            id: o.id.clone(),
                            text: o.text.clone(),
                        })
                        .collect(),
                    tok_s: mean_tok_s,
                };
                self.state
                    .baselines
                    .publish(baseline_ref, model, &new_baseline)
                    .await
                    .map_err(|e| ToolError::Execution(format!("run_eval baseline publish: {e}")))?;
                ("baseline_bootstrapped".to_string(), true)
            }
            (Some(base), false) => {
                let verdict = assess(self.state.embedder.as_ref(), &run.outputs, base, 0.87, 0.95)
                    .await
                    .map_err(|e| ToolError::Execution(format!("run_eval verdict: {e}")))?;
                (verdict_kind(&verdict), false)
            }
        };

        Ok(json!({
            "model": model,
            "baseline_ref": baseline_ref,
            "status": status,
            "baseline_recorded": recorded,
            "mean_tok_s": mean_tok_s,
            "outputs": outputs_json,
        }))
    }
}

fn verdict_kind(status: &crate::status::TypedStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

/// Register the eval tools into a ToolRegistry (codebase pattern).
pub fn register_tools(registry: &mut arkavo_mcp_tools::ToolRegistry, state: Arc<EvalState>) {
    registry.register("run_eval", Box::new(RunEvalTool::new(state)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::{OperatorError, PromptOutput, RunOutput};
    use crate::verdict::VerdictError;

    struct FakeOp(String);
    #[async_trait]
    impl Operator for FakeOp {
        async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError> {
            Ok(RunOutput {
                outputs: plan
                    .prompts
                    .iter()
                    .map(|p| PromptOutput {
                        id: p.id.clone(),
                        text: self.0.clone(),
                        tok_s: 10.0,
                    })
                    .collect(),
            })
        }
    }

    struct FakeEmb;
    #[async_trait]
    impl Embedder for FakeEmb {
        async fn embed(&self, t: &str) -> Result<Vec<f32>, VerdictError> {
            let mut v = vec![0.0f32; 27];
            for c in t.to_lowercase().chars() {
                if c.is_ascii_lowercase() {
                    v[(c as u8 - b'a') as usize] += 1.0;
                } else {
                    v[26] += 1.0;
                }
            }
            Ok(v)
        }
    }

    #[tokio::test]
    async fn bootstrap_then_pass_via_building_blocks() {
        use crate::baseline_file::FileBaselineStore;
        let dir = std::env::temp_dir().join(format!("arkavo-eval-tool-{}", std::process::id()));
        let store = FileBaselineStore::new(&dir);
        let plan = EvalPlan {
            model: ModelSpec {
                name: "m".into(),
                quant: "q".into(),
                weight_digest: "b3:0".into(),
            },
            prompts: EvalState::default_prompts(),
            exec: ExecutionProfile {
                seed: 0,
                temperature: 0.0,
                threads: None,
                ctx: None,
                max_tokens: 8,
            },
            baseline_commit: Some("main".into()),
        };
        let r1 = FakeOp("paris".into()).run(&plan).await.unwrap();
        let base = Baseline {
            outputs: r1
                .outputs
                .iter()
                .map(|o| BaselineOutput {
                    id: o.id.clone(),
                    text: o.text.clone(),
                })
                .collect(),
            tok_s: 10.0,
        };
        store.publish("main", "m", &base).await.unwrap();
        let r2 = FakeOp("paris".into()).run(&plan).await.unwrap();
        let fetched = store.fetch("main", "m").await.unwrap().unwrap();
        let status = assess(&FakeEmb, &r2.outputs, &fetched, 0.87, 0.95)
            .await
            .unwrap();
        assert_eq!(verdict_kind(&status), "passed");
        std::fs::remove_dir_all(dir).ok();
    }
}
