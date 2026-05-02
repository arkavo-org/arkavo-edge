//! Single-step execution: dispatches each command in a step through the
//! router, accumulates token usage, and threads a per-step reasoning trace
//! across commands so the model retains context within the step.

use crate::cognitive_engine_types::PlanStep;
use crate::error::{Error, Result};
use crate::step_context::StepTrace;
use crate::token_estimator;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::Router;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, info};

/// Execute every command in `step` in order, returning the cumulative
/// token usage. Each individual command is wrapped in `command_timeout`
/// so a single hung provider call cannot block the whole step.
///
/// `trace` is rebuilt fresh per step (no caller state required); kept as
/// a method-local instead of an argument to keep the call site simple.
pub async fn do_step(
    router: &Arc<Router>,
    tool_registry: &Arc<ToolRegistry>,
    step: &PlanStep,
    command_timeout: Duration,
) -> Result<u32> {
    debug!(step = step.step_number, "Executing step");

    let mut tokens_used = 0u32;
    // C5: rolling reasoning trace across commands within this step.
    let mut trace = StepTrace::new();

    for command in &step.commands {
        debug!(command, "Executing command");

        let task_prompt = format!(
            "Execute this command as part of fixing a GitHub issue:\n\
            Step {}: {}\n\
            Command: {}\n\n\
            Use the available tools to complete this task. \
            You have access to filesystem, git, and github tools.",
            step.step_number, step.description, command
        );

        // C5: prepend the rolling trace (if any) to the prompt.
        let messages = trace.build_messages(task_prompt.clone());

        // R4: bound a single command's wall-clock time.
        let call = router.route_with_tools(command, messages, Some(tool_registry));
        let response = match timeout(command_timeout, call).await {
            Err(_) => {
                return Err(Error::Other(anyhow::anyhow!(
                    "step {} command timed out after {}s: {}",
                    step.step_number,
                    command_timeout.as_secs(),
                    command
                )));
            }
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                return Err(Error::Other(anyhow::anyhow!(
                    "Command execution failed: {e}"
                )));
            }
        };

        info!(
            step = step.step_number,
            tool_calls = response.tool_calls.len(),
            "Command executed with {} tool calls",
            response.tool_calls.len()
        );

        // P4: provider-reported counts when available; otherwise estimate
        // against the FULL prompt actually sent (the rendered trace +
        // task_prompt) so the fallback path doesn't undercount when the
        // trace is non-empty.
        let accounted_prompt = trace.rendered_prompt(&task_prompt);
        let (input_tokens, output_tokens) =
            token_estimator::tokens_from_response(&accounted_prompt, &response);
        tokens_used = tokens_used
            .saturating_add(input_tokens)
            .saturating_add(output_tokens);

        if !response.tool_calls.is_empty() {
            debug!(
                "Tool calls executed: {:?}",
                response
                    .tool_calls
                    .iter()
                    .map(|tc| &tc.tool_name)
                    .collect::<Vec<_>>()
            );
        }

        // C5: record this command's outcome so the next command starts
        // with context.
        trace.record(command, &response);
    }

    Ok(tokens_used)
}
