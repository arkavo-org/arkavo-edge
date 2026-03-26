//! Three-track parallel agent loop: Plan / Execute / Judge
//!
//! Splits the sequential tool loop into concurrent tracks:
//! - Planner (large model): produces batched tool call plans
//! - Executor (mid model): executes tool calls via MCP
//! - Judge (small model): distills results, scores quality, synthesizes feedback

use super::conductor_tool_loop::{ToolLoopResult, distill_with_small_model};
use super::learning_bus::{LearningBus, LearningEvent};
use super::tool_memory::ToolMemory;
use arkavo_llm::ParsedToolCall;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::mcp_registry::McpRegistry;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, info, warn};

/// Batch of tool calls from the planner
struct PlannedActions {
    tool_calls: Vec<ParsedToolCall>,
}

/// Result of executing a single tool call
struct ExecutionResult {
    tool_name: String,
    result: String,
    success: bool,
    reward: Option<f64>,
}

/// Feedback from the judge to the planner
struct JudgeFeedback {
    distilled_results: String,
    should_replan: bool,
}

/// Run the three-track parallel tool loop for orchestrators.
///
/// The planner generates batched actions on the hinted model (large).
/// The executor runs tool calls on a fast model (3B).
/// The judge distills results and provides feedback on the smallest model (0.8B).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_tool_loop_parallel(
    router: &Arc<arkavo_router::Router>,
    registry_arc: &Arc<ToolRegistry>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: &str,
    messages: Vec<arkavo_llm::Message>,
    model_hint: Option<&arkavo_router::ModelChoice>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<RwLock<ToolMemory>>>,
    compute_budget: Option<&arkavo_budget::SharedComputeBudget>,
) -> Result<ToolLoopResult, String> {
    let loop_start = std::time::Instant::now();

    let (plan_tx, plan_rx) = mpsc::channel::<PlannedActions>(4);
    let (result_tx, result_rx) = mpsc::channel::<ExecutionResult>(16);
    let (feedback_tx, feedback_rx) = mpsc::channel::<JudgeFeedback>(4);

    // Spawn executor track (3B model, chat_semaphore)
    let exec_router = router.clone();
    let exec_registry = registry_arc.clone();
    let exec_mcp = mcp_registry.clone();
    let exec_bus = learning_bus.cloned();
    let exec_mem = tool_memory.cloned();
    let executor = tokio::spawn(async move {
        executor_track(
            &exec_router,
            &exec_registry,
            &exec_mcp,
            plan_rx,
            result_tx,
            exec_bus.as_ref(),
            exec_mem.as_ref(),
        )
        .await;
    });

    // Spawn judge track (0.8B model, synthesis semaphore)
    let judge_router = router.clone();
    let judge =
        tokio::spawn(async move { judge_track(&judge_router, result_rx, feedback_tx).await });

    // Run planner on current task (blocking — drives the loop)
    let plan_result = planner_track(
        router,
        registry_arc,
        mcp_registry,
        task_content,
        messages,
        model_hint,
        plan_tx,
        feedback_rx,
        compute_budget,
    )
    .await;

    // Wait for executor and judge to drain
    let _ = executor.await;
    let _ = judge.await;

    let total_latency = loop_start.elapsed().as_millis() as u64;

    Ok(ToolLoopResult {
        final_text: plan_result.final_text,
        decision_model_name: plan_result.decision_model_name,
        total_latency_ms: total_latency,
        context_tokens: plan_result.context_tokens,
        context_utilization_pct: plan_result.context_utilization_pct,
        inference_timing: plan_result.inference_timing,
        tool_call_count: plan_result.tool_call_count,
    })
}

struct PlanResult {
    final_text: String,
    decision_model_name: Option<String>,
    context_tokens: u32,
    context_utilization_pct: f64,
    inference_timing: Option<arkavo_llm::provider::InferenceTiming>,
    tool_call_count: usize,
}

/// Planner track: runs on the hinted (large) model.
/// Produces batched tool call plans, reads judge feedback between rounds.
#[allow(clippy::too_many_arguments)]
async fn planner_track(
    router: &Arc<arkavo_router::Router>,
    registry_arc: &Arc<ToolRegistry>,
    _mcp_registry: &Arc<McpRegistry>,
    task_content: &str,
    mut messages: Vec<arkavo_llm::Message>,
    model_hint: Option<&arkavo_router::ModelChoice>,
    plan_tx: mpsc::Sender<PlannedActions>,
    mut feedback_rx: mpsc::Receiver<JudgeFeedback>,
    compute_budget: Option<&arkavo_budget::SharedComputeBudget>,
) -> PlanResult {
    let mut result = PlanResult {
        final_text: String::new(),
        decision_model_name: None,
        context_tokens: 0,
        context_utilization_pct: 0.0,
        inference_timing: None,
        tool_call_count: 0,
    };

    let model_ctx = super::rlm_bridge::model_context_size(model_hint.map(|h| h.name()), false);

    for plan_round in 0..3 {
        // Consume any judge feedback from previous round
        while let Ok(feedback) = feedback_rx.try_recv() {
            if !feedback.distilled_results.is_empty() {
                messages.push(arkavo_llm::Message::user(format!(
                    "Previous results:\n{}",
                    feedback.distilled_results
                )));
            }
            if feedback.should_replan {
                messages.push(arkavo_llm::Message::user(
                    "Previous actions had negative results. Adjust strategy.".to_string(),
                ));
            }
        }

        // Check compute budget
        if let Some(budget) = compute_budget {
            let snap = budget.read().await.snapshot();
            if !snap.has_remaining {
                info!("Planner: compute budget exhausted at round {plan_round}");
                break;
            }
        }

        let context_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let context_tokens = context_chars / 4;
        result.context_tokens = result.context_tokens.max(context_tokens as u32);
        if model_ctx > 0 {
            result.context_utilization_pct = (context_tokens as f64 / model_ctx as f64 * 100.0)
                .max(result.context_utilization_pct);
        }

        let timeout_secs = match model_hint {
            Some(h) if h.size_bytes() >= 15_000_000_000 => 240u64,
            Some(h) if h.size_bytes() >= 5_000_000_000 => 180,
            _ => 90,
        };

        info!(
            plan_round,
            context_tokens,
            timeout_secs,
            model = model_hint.map(|h| h.name()).unwrap_or("auto"),
            "Planner: starting inference"
        );

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            router.route_with_tools_hinted(
                task_content,
                messages.clone(),
                Some(registry_arc),
                model_hint,
            ),
        )
        .await;

        let response = match response {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                warn!("Planner inference failed: {e}");
                break;
            }
            Err(_) => {
                warn!("Planner inference timed out at {timeout_secs}s");
                break;
            }
        };

        if let Some(ref timing) = response.inference_timing
            && result.inference_timing.is_none()
        {
            result.inference_timing = Some(timing.clone());
        }
        result.decision_model_name = router.last_routed_model();
        result.final_text = response.content.clone();

        if response.tool_calls.is_empty() {
            info!("Planner round {plan_round}: no tool calls, done");
            break;
        }

        let call_count = response.tool_calls.len();
        result.tool_call_count += call_count;
        info!("Planner round {plan_round}: produced {call_count} tool calls");

        messages.push(arkavo_llm::Message::assistant(response.content.clone()));

        if plan_tx
            .send(PlannedActions {
                tool_calls: response.tool_calls,
            })
            .await
            .is_err()
        {
            warn!("Planner: executor channel closed");
            break;
        }
    }

    // Drop sender to signal executor to stop
    drop(plan_tx);
    result
}

/// Executor track: runs tool calls from the planner.
/// Sends results to the judge for distillation and feedback.
async fn executor_track(
    router: &Arc<arkavo_router::Router>,
    registry_arc: &Arc<ToolRegistry>,
    mcp_registry: &Arc<McpRegistry>,
    mut plan_rx: mpsc::Receiver<PlannedActions>,
    result_tx: mpsc::Sender<ExecutionResult>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<RwLock<ToolMemory>>>,
) {
    let mut step_idx: usize = 0;

    while let Some(planned) = plan_rx.recv().await {
        info!(
            tool_count = planned.tool_calls.len(),
            "Executor: received action batch"
        );

        for tool_call in &planned.tool_calls {
            // Skip setup tools that already succeeded (e.g., registerAgent)
            if let Some(mem) = tool_memory
                && mem.read().await.is_setup_complete(&tool_call.tool_name)
            {
                info!(
                    "Executor: skipping {} (already completed)",
                    tool_call.tool_name
                );
                let _ = result_tx
                    .send(ExecutionResult {
                        tool_name: tool_call.tool_name.clone(),
                        result: "Already completed — skipped".to_string(),
                        success: true,
                        reward: None,
                    })
                    .await;
                continue;
            }

            let args = tool_call.arguments.clone();
            let start = std::time::Instant::now();

            let tool_result = if let Some(tool) = registry_arc.get(&tool_call.tool_name) {
                tool.execute(args.clone()).await.map_err(|e| e.to_string())
            } else {
                mcp_registry
                    .call_tool(&tool_call.tool_name, args.clone(), "hrm-parallel")
                    .await
                    .map_err(|e| e.to_string())
            };

            let latency_ms = start.elapsed().as_millis() as u64;

            match tool_result {
                Ok(result_val) => {
                    let result_str = serde_json::to_string(&result_val).unwrap_or_default();
                    let reward = super::conductor::extract_reward_from_result(&result_str);
                    let success = reward.is_none_or(|r| r >= 0.0);

                    if let Some(r) = reward
                        && r < 0.0
                    {
                        info!(
                            "Executor: {} returned negative reward {:.3}",
                            tool_call.tool_name, r
                        );
                    }

                    info!(
                        "Executor: {} succeeded ({}ms)",
                        tool_call.tool_name, latency_ms
                    );

                    // Record in tool memory
                    if let Some(mem) = tool_memory {
                        mem.write()
                            .await
                            .add(tool_call.tool_name.clone(), &args, &result_str);
                    }

                    // Send learning event
                    if let Some(bus) = learning_bus {
                        let event = LearningEvent::ToolCall {
                            tool_name: tool_call.tool_name.clone(),
                            args: args.clone(),
                            result: result_str.clone(),
                            success,
                            latency_ms,
                            decision_trace_id: None,
                            step_index: step_idx as u16,
                            model_name: router.last_routed_model(),
                        };
                        let _ = bus.sender().send(event).await;
                    }

                    let _ = result_tx
                        .send(ExecutionResult {
                            tool_name: tool_call.tool_name.clone(),
                            result: result_str,
                            success,
                            reward,
                        })
                        .await;
                }
                Err(err) => {
                    warn!("Executor: {} failed: {err}", tool_call.tool_name);

                    if let Some(mem) = tool_memory {
                        mem.write().await.add(
                            tool_call.tool_name.clone(),
                            &args,
                            &format!("Error: {err}"),
                        );
                    }

                    let _ = result_tx
                        .send(ExecutionResult {
                            tool_name: tool_call.tool_name.clone(),
                            result: format!("Error: {err}"),
                            success: false,
                            reward: None,
                        })
                        .await;
                }
            }

            step_idx += 1;
        }
    }

    info!("Executor: channel closed, shutting down");
}

/// Judge track: distills execution results and provides feedback to the planner.
/// Uses the smallest model (0.8B) via route_fast for synthesis.
async fn judge_track(
    router: &Arc<arkavo_router::Router>,
    mut result_rx: mpsc::Receiver<ExecutionResult>,
    feedback_tx: mpsc::Sender<JudgeFeedback>,
) {
    let mut batch_results = Vec::new();
    let mut has_negative_reward = false;

    while let Some(exec_result) = result_rx.recv().await {
        debug!(
            "Judge: received result for {} (success={}, reward={:?})",
            exec_result.tool_name, exec_result.success, exec_result.reward
        );

        if let Some(r) = exec_result.reward
            && r < 0.0
        {
            has_negative_reward = true;
        }

        // Distill large results
        let distilled = if exec_result.result.len() > 1500 {
            match distill_with_small_model(router, &exec_result.result).await {
                Some(summary) => {
                    info!(
                        raw_len = exec_result.result.len(),
                        summary_len = summary.len(),
                        "Judge: distilled {} result",
                        exec_result.tool_name
                    );
                    summary
                }
                None => {
                    // Structural fallback: first 500 chars
                    exec_result.result.chars().take(500).collect()
                }
            }
        } else {
            exec_result.result.clone()
        };

        batch_results.push(format!(
            "{}: {}",
            exec_result.tool_name,
            if distilled.len() > 200 {
                format!("{}...", &distilled[..200])
            } else {
                distilled
            }
        ));

        // Send feedback after accumulating a batch or on negative reward
        if batch_results.len() >= 3 || has_negative_reward {
            let feedback = JudgeFeedback {
                distilled_results: batch_results.join("\n"),
                should_replan: has_negative_reward,
            };

            if feedback_tx.send(feedback).await.is_err() {
                break; // planner closed
            }

            batch_results.clear();
            has_negative_reward = false;
        }
    }

    // Flush remaining
    if !batch_results.is_empty() {
        let _ = feedback_tx
            .send(JudgeFeedback {
                distilled_results: batch_results.join("\n"),
                should_replan: has_negative_reward,
            })
            .await;
    }

    info!("Judge: channel closed, shutting down");
}
