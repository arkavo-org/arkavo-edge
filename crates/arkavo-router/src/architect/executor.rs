use super::{ArchitectPlan, ArchitectResult, Subtask};
use crate::decision::ModelChoice;
use crate::{Error, Result, Router};
use arkavo_llm::{Message, Provider, ProviderResponse, Role};
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
    _router: Arc<Router>,
    max_retries: u8,
}

impl ArchitectExecutor {
    pub fn new(router: Arc<Router>) -> Self {
        Self {
            _router: router,
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
        accumulated_context.push(Message {
            role: Role::User,
            content: format!(
                "We are executing a multi-step plan for: {}\n\nI will give you one subtask at a time.",
                plan.original_task
            ),
            images: None,
        });

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
                accumulated_context.push(Message {
                    role: Role::Assistant,
                    content: format!(
                        "Completed subtask {}: {}\n\nResult:\n{}",
                        subtask.index, subtask.description, result.response
                    ),
                    images: None,
                });
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
        #[allow(unused_assignments)]
        let mut last_error = None;
        let mut current_model = subtask.assigned_model.clone();

        loop {
            let provider = self.get_provider(&current_model).await?;

            // Build subtask prompt
            let mut messages = context.to_vec();
            messages.push(Message {
                role: Role::User,
                content: format!(
                    "Execute this subtask:\n\n{}\n\nProvide a complete implementation.",
                    subtask.description
                ),
                images: None,
            });

            // Execute with or without tools
            let response = if tool_registry.is_some() && provider.supports_tools() {
                let tools_json = tool_registry.map(|r| {
                    let tool_infos = r.list_tools();
                    // Use the correct format based on the model provider
                    match current_model {
                        ModelChoice::GeminiFlash | ModelChoice::GeminiPro => {
                            arkavo_llm::McpConverter::to_gemini_format(&tool_infos)
                        }
                        _ => arkavo_llm::McpConverter::to_anthropic_format(&tool_infos),
                    }
                });

                provider
                    .complete_with_tools(messages, tools_json, None)
                    .await
            } else {
                provider
                    .complete(messages)
                    .await
                    .map(|content| ProviderResponse {
                        content,
                        reasoning_content: None,
                        tool_calls: Vec::new(),
                        finish_reason: Some("stop".to_string()),
                    })
            };

            match response {
                Ok(resp) => {
                    let cost = self.estimate_actual_cost(&current_model, &resp);
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
                    let error_msg = e.to_string();
                    last_error = Some(error_msg.clone());
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
                        break;
                    }

                    // Escalate to more capable model
                    current_model = self.escalate_model(&current_model);
                    tracing::warn!(
                        subtask_index = subtask.index,
                        new_model = ?current_model,
                        previous_error = %error_msg,
                        "Escalating to more capable model after failure"
                    );
                }
            }
        }

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
            error: last_error,
            retry_count,
        })
    }

    async fn get_provider(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        match model {
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus => {
                use arkavo_llm::providers::anthropic::AnthropicProvider;
                AnthropicProvider::from_env()
                    .map(|p| Box::new(p) as Box<dyn Provider>)
                    .map_err(|e| {
                        Error::ModelExecution(format!("Failed to create Anthropic provider: {e}"))
                    })
            }
            ModelChoice::GeminiFlash | ModelChoice::GeminiPro => arkavo_llm::GeminiProvider::new()
                .map(|p| Box::new(p) as Box<dyn Provider>)
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create Gemini provider: {e}"))
                }),
            _ => {
                // For local models, use Gemini as fallback if available
                if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                    return Ok(Box::new(provider));
                }
                Err(Error::ModelExecution(format!(
                    "Model {model:?} not available"
                )))
            }
        }
    }

    fn escalate_model(&self, current: &ModelChoice) -> ModelChoice {
        match current {
            // Qwen3/Ministral escalation path
            ModelChoice::LocalQwen3 => ModelChoice::LocalMinistral3B,
            ModelChoice::LocalMinistral3B => ModelChoice::LocalMinistral8B,
            ModelChoice::LocalMinistral8B => ModelChoice::LocalQwen35_27B,
            ModelChoice::LocalQwen35_27B => ModelChoice::LocalGlm47Flash,
            ModelChoice::LocalGlm47Flash => ModelChoice::GeminiFlash,
            // Legacy Gemma escalation path
            ModelChoice::LocalGemma270M => ModelChoice::LocalGemma4B,
            ModelChoice::LocalGemma4B => ModelChoice::GeminiFlash,
            ModelChoice::LocalGemma12B => ModelChoice::GeminiPro,
            // Other escalation paths
            ModelChoice::LocalDeepSeekCoder => ModelChoice::DeepSeekV32,
            ModelChoice::DeepSeekV32 => ModelChoice::ClaudeSonnet,
            ModelChoice::DeepSeekV32Speciale => ModelChoice::ClaudeOpus,
            ModelChoice::GeminiFlash => ModelChoice::ClaudeSonnet,
            ModelChoice::ClaudeSonnet => ModelChoice::GeminiPro,
            ModelChoice::GeminiPro => ModelChoice::ClaudeOpus,
            ModelChoice::ClaudeOpus => ModelChoice::ClaudeOpus,
            ModelChoice::KimiK2 => ModelChoice::ClaudeSonnet,
        }
    }

    fn estimate_actual_cost(&self, model: &ModelChoice, response: &ProviderResponse) -> f64 {
        // Rough estimation based on response length
        // Actual token counts would require tokenizer
        let output_tokens = (response.content.len() / 4) as f64; // ~4 chars per token
        let input_tokens = output_tokens / 3.0; // Typical ratio

        match model {
            ModelChoice::GeminiFlash => {
                (input_tokens / 1_000_000.0).mul_add(0.30, (output_tokens / 1_000_000.0) * 2.50)
            }
            ModelChoice::GeminiPro => {
                (input_tokens / 1_000_000.0).mul_add(1.25, (output_tokens / 1_000_000.0) * 5.00)
            }
            ModelChoice::ClaudeSonnet => {
                (input_tokens / 1_000_000.0).mul_add(3.00, (output_tokens / 1_000_000.0) * 15.00)
            }
            ModelChoice::ClaudeOpus => {
                (input_tokens / 1_000_000.0).mul_add(15.00, (output_tokens / 1_000_000.0) * 75.00)
            }
            _ => 0.0,
        }
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

    #[test]
    fn test_model_escalation() {
        let router = futures::executor::block_on(async { Router::new().await.ok() });

        if let Some(router) = router {
            let executor = ArchitectExecutor::new(Arc::new(router));

            assert_eq!(
                executor.escalate_model(&ModelChoice::GeminiFlash),
                ModelChoice::ClaudeSonnet
            );
            assert_eq!(
                executor.escalate_model(&ModelChoice::ClaudeSonnet),
                ModelChoice::GeminiPro
            );
            assert_eq!(
                executor.escalate_model(&ModelChoice::ClaudeOpus),
                ModelChoice::ClaudeOpus
            );
        }
    }
}
