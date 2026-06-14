//! The Eval Task Contract: the single source of truth for what an eval runs.
//! Committed to the repo and content-addressed; references models/baselines by
//! `b3:<hex>` digest, never a mutable key.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalContract {
    pub contract_id: String,
    /// Always "model_eval" for this pipeline.
    pub task_kind: String,
    pub model: ModelSpec,
    pub baseline: BaselineRef,
    pub prompts: Vec<EvalPrompt>,
    pub acceptance: Acceptance,
    pub execution: ExecutionProfile,
    /// Names of required preconditions, e.g. ["weights_present","baseline_present"].
    pub preconditions: Vec<String>,
    /// torg circuit reference, e.g. "torg:eval-preflight-v1".
    pub policy_circuit: String,
    /// "refuse" is the only supported value in this slice.
    pub on_precondition_unmet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    pub quant: String,
    /// `b3:<hex>` of the GGUF weights.
    pub weight_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineRef {
    /// "reference_outputs".
    pub kind: String,
    /// Git commit the baseline is anchored to (the lookup key). Optional on the
    /// very first run before any baseline exists.
    pub commit: Option<String>,
    /// Resolved `b3:<hex>` of the baseline artifact, if known.
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalPrompt {
    pub id: String,
    pub messages: Vec<PromptMessage>,
    /// Optional tool definitions (serde_json array) for tool-calling prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Acceptance {
    /// Minimum aggregate cosine similarity vs baseline (e.g. 0.87).
    pub min_similarity: f64,
    /// Minimum tok/s as a fraction of the baseline tok/s (e.g. 0.95).
    pub min_tok_s_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionProfile {
    pub seed: u32,
    pub temperature: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx: Option<u32>,
    pub max_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EvalContract {
        EvalContract {
            contract_id: "eval/gemma-4-12b/abc123".into(),
            task_kind: "model_eval".into(),
            model: ModelSpec {
                name: "gemma-4-12b".into(),
                quant: "Q4_K_M".into(),
                weight_digest: "b3:".to_string() + &"0".repeat(64),
            },
            baseline: BaselineRef {
                kind: "reference_outputs".into(),
                commit: Some("main0".into()),
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
                max_tokens: 64,
            },
            preconditions: vec!["weights_present".into(), "baseline_present".into()],
            policy_circuit: "torg:eval-preflight-v1".into(),
            on_precondition_unmet: "refuse".into(),
        }
    }

    #[test]
    fn json_round_trip() {
        let c = sample();
        let json = serde_json::to_string(&c).unwrap();
        let back: EvalContract = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
