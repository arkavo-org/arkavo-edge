use super::{ArchitectPlan, ArchitectResult, Subtask};
use crate::decision::ModelChoice;
use crate::{Error, Result, Router};
use arkavo_llm::{Message, ProviderResponse};
use arkavo_mcp_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Result of executing a single subtask
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskResult {
    /// Subtask ID
    pub subtask_id: Uuid,
    /// Subtask index
    pub index: usize,
    /// Model that executed this subtask
    pub model_used: ModelChoice,
    /// Raw response content
    pub response: String,
    /// Reasoning/thinking content from thinking models (e.g., DeepSeek V3.2-Speciale)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Tool calls made (if any)
    pub tool_calls: Vec<serde_json::Value>,
    /// Actual cost incurred
    pub actual_cost_usd: f64,
    /// Whether execution succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Number of retry attempts
    pub retry_count: u8,
}

/// Executes architect plans by routing subtasks to appropriate models
pub struct ArchitectExecutor {
    router: Arc<Router>,
    max_retries: u8,
}

impl ArchitectExecutor {
    pub fn new(router: Arc<Router>) -> Self {
        Self {
            router,
            max_retries: 2,
        }
    }

    /// Execute an architect plan, routing each subtask to the appropriate model
    pub async fn execute(
        &self,
        plan: &ArchitectPlan,
        context: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<ArchitectResult> {
        let mut results = Vec::new();
        let mut total_cost = 0.0;

        // Build context with original task
        let mut accumulated_context = context;
        accumulated_context.push(Message::user(format!(
            "We are executing a multi-step plan for: {}\n\nI will give you one subtask at a time.",
            plan.original_task
        )));

        // Execute subtasks in order (respecting dependencies)
        for subtask in &plan.subtasks {
            // Check dependencies are satisfied
            for dep_idx in &subtask.dependencies {
                if *dep_idx < results.len() {
                    let dep_result: &SubtaskResult = &results[*dep_idx];
                    if !dep_result.success {
                        return Err(Error::ModelExecution(format!(
                            "Subtask {} depends on failed subtask {}",
                            subtask.index, dep_idx
                        )));
                    }
                }
            }

            // Execute the subtask
            let result = self
                .execute_subtask(subtask, &accumulated_context, tool_registry)
                .await?;

            // Add successful result to context for subsequent subtasks
            if result.success {
                accumulated_context.push(Message::assistant(format!(
                    "Completed subtask {}: {}\n\nResult:\n{}",
                    subtask.index, subtask.description, result.response
                )));
            }

            total_cost += result.actual_cost_usd;
            results.push(result);
        }

        // Synthesize final response
        let final_response = self.synthesize_results(plan, &results).await?;

        let actual_savings = plan.opus_only_estimate_usd - total_cost;
        let was_cost_effective = actual_savings > 0.0;

        Ok(ArchitectResult {
            plan: plan.clone(),
            subtask_results: results,
            final_response,
            actual_cost_usd: total_cost,
            actual_savings_usd: actual_savings,
            was_cost_effective,
        })
    }

    async fn execute_subtask(
        &self,
        subtask: &Subtask,
        context: &[Message],
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<SubtaskResult> {
        let mut retry_count = 0;
        let mut current_model = subtask.assigned_model.clone();
        let budget = self.router.call_budget();

        let last_error = loop {
            // Build subtask prompt
            let mut messages = context.to_vec();
            messages.push(Message::user(format!(
                "Execute this subtask:\n\n{}\n\nProvide a complete implementation.",
                subtask.description
            )));

            let tools_json = tool_registry.map(|r| {
                let tool_infos = r.list_tools();
                // Use the correct format based on the model provider
                match current_model {
                    ModelChoice::GeminiFlash
                    | ModelChoice::Gemini35Flash
                    | ModelChoice::GeminiPro => {
                        arkavo_llm::McpConverter::to_gemini_format(&tool_infos)
                    }
                    _ => arkavo_llm::McpConverter::to_anthropic_format(&tool_infos),
                }
            });

            let preflight = crate::usage::estimate_request(&messages, tools_json.as_ref(), 4096);
            let settled = crate::usage::estimate_request(&messages, tools_json.as_ref(), 0);
            // Both gates run before the provider exists: a refusal never opens
            // a client, and an exhausted budget stops the plan rather than
            // starting another paid attempt. The ledger answers first so an
            // exhausted budget reports as `BudgetExceeded` rather than as a
            // policy denial. Subtask arms are planner-assigned, never named by
            // the caller, so the cloud gate is asked without authorization.
            let estimated_cost = self.router.usage_cost(&current_model, &preflight);
            if let Some(budget) = budget {
                budget.check(estimated_cost).await?;
            }
            self.router
                .authorize_call(&current_model, estimated_cost, false)
                .await?;

            let (provider, _) = self.router.get_provider_attributed(&current_model).await?;
            let use_tools = tools_json.is_some() && provider.supports_tools();
            let result = if use_tools {
                provider
                    .complete_with_tools(messages, tools_json, None)
                    .await
            } else {
                provider
                    .complete_with_schema_response(messages, None, None)
                    .await
            };

            // Settle every attempt — a rejected retry stays charged.
            let response = self
                .router
                .account_result(&current_model, &settled, result, budget)
                .await;

            match response {
                Ok(resp) => {
                    let cost = self.attempt_cost(&current_model, &settled, &resp);
                    return Ok(SubtaskResult {
                        subtask_id: subtask.id,
                        index: subtask.index,
                        model_used: current_model,
                        response: resp.content,
                        reasoning_content: resp.reasoning_content,
                        tool_calls: resp
                            .tool_calls
                            .iter()
                            .filter_map(|tc| serde_json::to_value(tc).ok())
                            .collect(),
                        actual_cost_usd: cost,
                        success: true,
                        error: None,
                        retry_count,
                    });
                }
                Err(e) => {
                    // A ledger failure is not a model failure: retrying would
                    // spend again against a budget that already refused.
                    if matches!(e, Error::BudgetExceeded(_) | Error::BudgetError(_)) {
                        return Err(e);
                    }
                    let error_msg = e.to_string();
                    retry_count += 1;

                    // Log the actual error for debugging
                    tracing::error!(
                        subtask_index = subtask.index,
                        model = ?current_model,
                        error = %error_msg,
                        retry_count = retry_count,
                        "Subtask execution failed"
                    );

                    if retry_count > self.max_retries {
                        tracing::error!(
                            subtask_index = subtask.index,
                            total_retries = retry_count,
                            last_error = %error_msg,
                            "Subtask failed after max retries exhausted"
                        );
                        break error_msg;
                    }

                    // Re-dispatching the same model just re-spends on the same
                    // failure, and a paid rung must be reachable. Local rungs
                    // stay unfiltered so an uncached weight is fetched on demand.
                    let Some(next_model) = super::escalation::next_rung(&current_model)
                        .filter(|next| next.is_local() || self.router.is_model_available(next))
                    else {
                        break format!(
                            "{error_msg} (no available escalation target beyond {})",
                            current_model.name()
                        );
                    };
                    current_model = next_model;
                    tracing::warn!(
                        subtask_index = subtask.index,
                        new_model = ?current_model,
                        previous_error = %error_msg,
                        "Escalating to more capable model after failure"
                    );
                }
            }
        };

        // All retries exhausted
        Ok(SubtaskResult {
            subtask_id: subtask.id,
            index: subtask.index,
            model_used: current_model,
            response: String::new(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            actual_cost_usd: 0.0,
            success: false,
            error: Some(last_error),
            retry_count,
        })
    }

    /// Cost of one settled attempt, priced off the same request estimate the
    /// ledger used so the plan total and the budget entries agree.
    fn attempt_cost(
        &self,
        model: &ModelChoice,
        estimated: &arkavo_budget::cost::TokenUsage,
        response: &ProviderResponse,
    ) -> f64 {
        self.router
            .attribute_response(model.clone(), estimated, response)
            .cost_usd
    }

    async fn synthesize_results(
        &self,
        plan: &ArchitectPlan,
        results: &[SubtaskResult],
    ) -> Result<String> {
        // If all subtasks succeeded, combine their outputs
        let successful_results: Vec<&SubtaskResult> =
            results.iter().filter(|r| r.success).collect();

        if successful_results.is_empty() {
            return Err(Error::ModelExecution("All subtasks failed".to_string()));
        }

        // Build summary
        let mut summary = format!("## Completed: {}\n\n", plan.original_task);

        use std::fmt::Write;
        for (i, result) in successful_results.iter().enumerate() {
            let subtask = &plan.subtasks[result.index];
            let _ = write!(
                summary,
                "### Step {} - {}\n{}\n\n",
                i + 1,
                subtask.description,
                result.response
            );
        }

        // Add cost summary
        let total_cost: f64 = results.iter().map(|r| r.actual_cost_usd).sum();
        let savings = plan.opus_only_estimate_usd - total_cost;
        let savings_pct = if plan.opus_only_estimate_usd > 0.0 {
            (savings / plan.opus_only_estimate_usd) * 100.0
        } else {
            0.0
        };

        let _ = write!(
            summary,
            "---\n**Cost**: ${total_cost:.4} (saved ${savings:.4}, {savings_pct:.1}% vs Opus-only)\n"
        );

        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn grok_actual_cost_uses_list_rates_and_usage() {
        let router = Router::new_offline().await.unwrap();
        let executor = ArchitectExecutor::new(Arc::new(router));

        let resp = ProviderResponse {
            response_items: Vec::new(),
            content: "hello".into(),
            inference_timing: Some(arkavo_llm::InferenceTiming {
                n_cached_prompt_eval: None,
                n_cache_write_prompt_eval: None,
                n_prompt_eval: 1_000_000,
                n_eval: 500_000,
                ..Default::default()
            }),
            ..Default::default()
        };
        // $2/M in + $6/M out → 2 + 3 = $5.00
        let usage = arkavo_budget::cost::TokenUsage::default();
        let cost = executor.attempt_cost(&ModelChoice::Grok46, &usage, &resp);
        assert!(
            (cost - 5.0).abs() < 1e-9,
            "Grok actual cost should be $5.00 for 1M/0.5M tokens, got {cost}"
        );
        assert!(
            cost > 0.0,
            "Grok must not be treated as free in architect accounting"
        );

        // n_eval and n_thinking_eval are disjoint; cost must sum them once.
        let with_thinking = ProviderResponse {
            response_items: Vec::new(),
            content: "hello".into(),
            inference_timing: Some(arkavo_llm::InferenceTiming {
                n_cached_prompt_eval: None,
                n_cache_write_prompt_eval: None,
                n_prompt_eval: 0,
                n_eval: 200_000,
                n_thinking_eval: Some(300_000),
                ..Default::default()
            }),
            ..Default::default()
        };
        // 500k output tokens at $6/M → $3.00 (not $3.00 + $1.80 double-count)
        let thinking_cost = executor.attempt_cost(&ModelChoice::Grok46, &usage, &with_thinking);
        assert!(
            (thinking_cost - 3.0).abs() < 1e-9,
            "thinking tokens must not double-count; got {thinking_cost}"
        );
    }
}
