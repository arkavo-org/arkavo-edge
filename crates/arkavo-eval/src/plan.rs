//! Planner role: resolves a contract into a concrete execution plan.

use crate::contract::{EvalContract, EvalPrompt, ExecutionProfile, ModelSpec};

#[derive(Debug, Clone)]
pub struct EvalPlan {
    pub model: ModelSpec,
    pub prompts: Vec<EvalPrompt>,
    pub exec: ExecutionProfile,
    /// The git commit whose baseline this run compares against (if any).
    pub baseline_commit: Option<String>,
}

pub fn plan(contract: &EvalContract) -> EvalPlan {
    EvalPlan {
        model: contract.model.clone(),
        prompts: contract.prompts.clone(),
        exec: contract.execution.clone(),
        baseline_commit: contract.baseline.commit.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::*;

    #[test]
    fn plan_carries_model_prompts_exec_and_baseline() {
        let c = EvalContract {
            contract_id: "id".into(),
            task_kind: "model_eval".into(),
            model: ModelSpec {
                name: "gemma-4-12b".into(),
                quant: "Q4_K_M".into(),
                weight_digest: "b3:0".into(),
            },
            baseline: BaselineRef {
                kind: "reference_outputs".into(),
                commit: Some("c1".into()),
                digest: None,
            },
            prompts: vec![EvalPrompt {
                id: "p1".into(),
                messages: vec![],
                tools: None,
            }],
            acceptance: Acceptance {
                min_similarity: 0.87,
                min_tok_s_ratio: 0.95,
            },
            execution: ExecutionProfile {
                seed: 0,
                temperature: 0.0,
                threads: Some(4),
                ctx: Some(4096),
                max_tokens: 64,
            },
            preconditions: vec![],
            policy_circuit: "torg:x".into(),
            on_precondition_unmet: "refuse".into(),
        };
        let p = plan(&c);
        assert_eq!(p.model.name, "gemma-4-12b");
        assert_eq!(p.prompts.len(), 1);
        assert_eq!(p.exec.threads, Some(4));
        assert_eq!(p.baseline_commit.as_deref(), Some("c1"));
    }
}
