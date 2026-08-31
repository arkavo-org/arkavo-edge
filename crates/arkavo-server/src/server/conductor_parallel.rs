//! Three-track parallel agent loop: Plan / Execute / Judge
//!
//! Splits the sequential tool loop into concurrent tracks:
//! - Planner (large model): produces batched tool call plans
//! - Executor (mid model): executes tool calls via MCP
//! - Judge (small model): distills results, scores quality, synthesizes feedback

use super::conductor_tool_loop::{
    ToolCallObservation, ToolLoopResult, distill_with_small_model, tool_call_permitted,
};
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
    call_id: String,
    result: String,
    success: bool,
    reward: Option<f64>,
}

/// A single condensed tool result with metadata for proper message construction
struct CondensedToolResult {
    tool_name: String,
    call_id: String,
    content: String,
}

/// Feedback from the judge to the planner
struct JudgeFeedback {
    tool_results: Vec<CondensedToolResult>,
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
    granted_tools: Option<&std::collections::HashSet<String>>,
    #[cfg(feature = "taint")] egress: Option<Arc<super::egress_guard::EgressGuard>>,
) -> Result<ToolLoopResult, String> {
    let loop_start = std::time::Instant::now();

    let declared_scope: Vec<String> = registry_arc
        .list_tools()
        .iter()
        .map(|t| t.name.clone())
        .collect();

    let (plan_tx, plan_rx) = mpsc::channel::<PlannedActions>(4);
    let (result_tx, result_rx) = mpsc::channel::<ExecutionResult>(16);
    let (feedback_tx, feedback_rx) = mpsc::channel::<JudgeFeedback>(4);
    let (obs_tx, mut obs_rx) = mpsc::channel::<ToolCallObservation>(64);

    // Clone granted_tools into an owned HashSet so the spawned executor can own it.
    // None = no filtering (unspecialized agent), Some = least-privilege grant set.
    let exec_granted: Option<std::collections::HashSet<String>> = granted_tools.cloned();

    // Spawn executor track (3B model, chat_semaphore)
    let exec_router = router.clone();
    let exec_registry = registry_arc.clone();
    let exec_mcp = mcp_registry.clone();
    let exec_bus = learning_bus.cloned();
    let exec_mem = tool_memory.cloned();
    let exec_declared = declared_scope.clone();
    #[cfg(feature = "taint")]
    let exec_egress = egress.clone();
    let executor = tokio::spawn(async move {
        executor_track(
            &exec_router,
            &exec_registry,
            &exec_mcp,
            plan_rx,
            result_tx,
            exec_bus.as_ref(),
            exec_mem.as_ref(),
            &exec_declared,
            obs_tx,
            exec_granted.as_ref(),
            #[cfg(feature = "taint")]
            exec_egress,
        )
        .await;
    });

    // Spawn judge track — structural condensation for JSON (instant, no GPU),
    // LLM distillation for unstructured text (rare, uses synthesis_semaphore).
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

    // Drain observations forwarded by the executor track. The channel is
    // closed when executor_track returns, so this loop terminates.
    let mut tool_observations: Vec<ToolCallObservation> = Vec::new();
    while let Some(obs) = obs_rx.recv().await {
        tool_observations.push(obs);
    }

    let total_latency = loop_start.elapsed().as_millis() as u64;

    Ok(ToolLoopResult {
        final_text: plan_result.final_text,
        decision_model_name: plan_result.decision_model_name,
        total_latency_ms: total_latency,
        context_tokens: plan_result.context_tokens,
        context_utilization_pct: plan_result.context_utilization_pct,
        inference_timing: plan_result.inference_timing,
        tool_call_count: plan_result.tool_call_count,
        tool_observations,
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
    let mut prev_degenerate = false;

    for plan_round in 0..2 {
        // Consume any judge feedback from previous round as proper tool result messages
        while let Ok(feedback) = feedback_rx.try_recv() {
            for tr in &feedback.tool_results {
                messages.push(arkavo_llm::Message::tool_result(
                    &tr.content,
                    &tr.call_id,
                    &tr.tool_name,
                ));
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

        if prev_degenerate {
            messages.push(arkavo_llm::Message::user(
                "IMPORTANT: Previous response was degenerate (too many tool calls). Use at most 3 tool calls this round.".to_string(),
            ));
        }

        // Smart context compaction: distill old messages via fast model before
        // dropping them, preserving key insights. Same approach as tool loop.
        let char_budget = model_ctx * 4;
        let mut context_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        if context_chars > char_budget && messages.len() > 3 {
            let keep_recent = 2;
            let compactable = messages.len() - 1 - keep_recent;
            if compactable > 0 {
                let old_messages: Vec<String> = messages[1..=compactable]
                    .iter()
                    .map(|m| format!("[{:?}] {}", m.role, &m.content[..m.content.len().min(500)]))
                    .collect();
                let old_chars: usize = messages[1..=compactable]
                    .iter()
                    .map(|m| m.content.len())
                    .sum();

                let old_summary = old_messages.join("\n---\n");
                let summary =
                    super::conductor_tool_loop::distill_with_small_model(router, &old_summary)
                        .await;

                for _ in 0..compactable {
                    messages.remove(1);
                }

                if let Some(distilled) = summary {
                    info!(
                        old_msgs = compactable,
                        old_chars,
                        summary_chars = distilled.len(),
                        "Planner: context compacted via fast model"
                    );
                    messages.insert(
                        1,
                        arkavo_llm::Message::user(format!(
                            "[Previous context summary]: {distilled}"
                        )),
                    );
                } else {
                    let structural = format!(
                        "[Previous context: {compactable} messages ({old_chars} chars) compacted]"
                    );
                    info!(
                        old_msgs = compactable,
                        old_chars, "Planner: context compacted (structural fallback)"
                    );
                    messages.insert(1, arkavo_llm::Message::user(structural));
                }

                context_chars = messages.iter().map(|m| m.content.len()).sum();
            }
        }
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

        // Round 0: full planning profile (thinking on, full schema, 16K tokens)
        // Round 1+: execution profile via route_with_tools_execution
        //   (temp 0.1, thinking off, max 200 tokens — same model)
        let inference_fut: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = arkavo_router::Result<arkavo_llm::ProviderResponse>,
                    > + Send,
            >,
        > = if plan_round > 0 {
            // Round 1+: re-inject tools with compact schemas (no parameters).
            // The ChatTool conversion in llamacpp_provider strips empty schemas,
            // so the Jinja template renders tool names + descriptions only —
            // compact enough to avoid the 16K+ token expansion from full schemas.
            Box::pin(router.route_with_tools_execution(
                task_content,
                messages.clone(),
                Some(registry_arc),
                model_hint,
            ))
        } else if let Some(hint) = model_hint {
            Box::pin(router.route_with_tools_override(
                task_content,
                messages.clone(),
                Some(registry_arc),
                hint,
            ))
        } else {
            Box::pin(router.route_with_tools_hinted(
                task_content,
                messages.clone(),
                Some(registry_arc),
                None,
            ))
        };

        let response =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), inference_fut).await;

        let response = match response {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                warn!("Planner round {plan_round} failed: {e}");
                break;
            }
            Err(_) => {
                // Timeout means the GPU is still busy with the cancelled inference.
                // Retrying would block on the same semaphore. Break and let the
                // next cycle start fresh when the GPU is available.
                warn!("Planner round {plan_round} timed out at {timeout_secs}s");
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
            if plan_round == 0 {
                // Round 0 produced text but no tool calls. Retry on round 1
                // with an explicit instruction to use tools.
                warn!("Planner round 0: no tool calls, will retry with tool nudge");
                messages.push(arkavo_llm::Message::assistant(response.content.clone()));
                messages.push(arkavo_llm::Message::user(
                    "You MUST use a tool now. Pick the most appropriate tool and call it."
                        .to_string(),
                ));
                continue;
            }
            info!("Planner round {plan_round}: no tool calls, done");
            break;
        }

        let call_count = response.tool_calls.len();
        let batch_degenerate = is_degenerate_batch(&response.tool_calls);
        if batch_degenerate {
            warn!(
                "Planner round {plan_round}: degenerate batch ({} calls), will reduce budget next round",
                response.tool_calls.len()
            );
        }
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

        // Wait for executor/judge feedback before next round so tool results
        // are injected into the conversation. Without this, the planner races
        // ahead and generates the next inference without seeing whether the
        // previous tool calls succeeded — causing repeated registerAgent calls
        // and lost context.
        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            feedback_rx.recv(),
        )
        .await
        {
            Ok(Some(feedback)) => {
                // Push tool results with proper role so Jinja templates
                // (especially Gemma 4) render <|tool_response> tokens.
                for tr in &feedback.tool_results {
                    messages.push(arkavo_llm::Message::tool_result(
                        &tr.content,
                        &tr.call_id,
                        &tr.tool_name,
                    ));
                }
                if feedback.should_replan {
                    messages.push(arkavo_llm::Message::user(
                        "Previous actions had negative results. Adjust strategy.".to_string(),
                    ));
                }
            }
            Ok(None) => {
                warn!("Planner: judge channel closed before feedback");
                break;
            }
            Err(_) => {
                warn!("Planner: timed out waiting for executor feedback");
                break;
            }
        }

        prev_degenerate = batch_degenerate;
    }

    // Drop sender to signal executor to stop
    drop(plan_tx);
    result
}

/// Collapse identical tool calls (same name + same args) in a batch.
/// Keeps the first occurrence. Logs when duplicates are suppressed.
fn dedup_tool_calls(calls: Vec<ParsedToolCall>) -> Vec<ParsedToolCall> {
    let original_len = calls.len();
    let mut seen = std::collections::HashSet::new();
    let deduped: Vec<ParsedToolCall> = calls
        .into_iter()
        .filter(|c| seen.insert((c.tool_name.clone(), c.arguments.to_string())))
        .collect();

    if deduped.len() < original_len {
        info!(
            original = original_len,
            kept = deduped.len(),
            "Suppressed duplicate tool calls in batch"
        );
    }
    deduped
}

/// A batch is degenerate if it has more than 10 tool calls (before cap/dedup).
fn is_degenerate_batch(calls: &[ParsedToolCall]) -> bool {
    calls.len() > 10
}

/// Executor track: runs tool calls from the planner.
/// Sends results to the judge for distillation and feedback.
#[allow(clippy::too_many_arguments)]
async fn executor_track(
    router: &Arc<arkavo_router::Router>,
    registry_arc: &Arc<ToolRegistry>,
    mcp_registry: &Arc<McpRegistry>,
    mut plan_rx: mpsc::Receiver<PlannedActions>,
    result_tx: mpsc::Sender<ExecutionResult>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<RwLock<ToolMemory>>>,
    declared_scope: &[String],
    obs_tx: mpsc::Sender<ToolCallObservation>,
    granted_tools: Option<&std::collections::HashSet<String>>,
    #[cfg(feature = "taint")] egress: Option<Arc<super::egress_guard::EgressGuard>>,
) {
    let mut step_idx: usize = 0;

    while let Some(planned) = plan_rx.recv().await {
        // Filter calls to only those permitted by the grant set before
        // executing — deny at the boundary, never fabricate.
        let raw_calls = dedup_tool_calls(planned.tool_calls);
        let tool_calls: Vec<_> = raw_calls
            .into_iter()
            .filter(|tc| {
                if tool_call_permitted(&tc.tool_name, granted_tools) {
                    true
                } else {
                    warn!(
                        tool = %tc.tool_name,
                        "Parallel executor: tool call denied (least-privilege)"
                    );
                    false
                }
            })
            .collect();
        info!(
            tool_count = tool_calls.len(),
            "Executor: received action batch"
        );

        // Execute all tool calls in the batch concurrently
        let tool_futures: Vec<_> = tool_calls
            .iter()
            .enumerate()
            .map(|(idx, tool_call)| {
                let registry = registry_arc.clone();
                let mcp = mcp_registry.clone();
                let args = tool_call.arguments.clone();
                let name = tool_call.tool_name.clone();
                let call_id = tool_call
                    .call_id
                    .clone()
                    .unwrap_or_else(|| format!("call_{idx}"));
                let mem = tool_memory.cloned();
                #[cfg(feature = "taint")]
                let guard = egress.clone();
                async move {
                    // Skip setup tools that already succeeded (e.g., registerAgent)
                    if let Some(ref m) = mem
                        && m.read().await.is_setup_complete(&name)
                    {
                        return (
                            idx,
                            name,
                            call_id,
                            args,
                            Ok::<String, String>("Already completed \u{2014} skipped".to_string()),
                            true,
                            None,
                            0u64,
                        );
                    }
                    // SEQ-003: refuse before dispatch. Calls in one batch run
                    // concurrently, so a call is judged against what the session
                    // held when the batch was planned — a same-batch peer's
                    // result cannot have reached these params yet.
                    #[cfg(feature = "taint")]
                    if let Some(ref g) = guard
                        && let Err(message) = g.check_call(&name, &args)
                    {
                        return (
                            idx,
                            name,
                            call_id,
                            args,
                            Err::<String, String>(message),
                            false,
                            None,
                            0u64,
                        );
                    }
                    let start = std::time::Instant::now();
                    let result = if let Some(tool) = registry.get(&name) {
                        tool.execute(args.clone()).await.map_err(|e| e.to_string())
                    } else {
                        mcp.call_tool(&name, args.clone(), "hrm-parallel")
                            .await
                            .map_err(|e| e.to_string())
                    };
                    let latency = start.elapsed().as_millis() as u64;
                    match result {
                        Ok(val) => {
                            let s = serde_json::to_string(&val).unwrap_or_default();
                            let reward = super::conductor::extract_reward_from_result(&s);
                            let success = reward.is_none_or(|r| r >= 0.0);
                            (idx, name, call_id, args, Ok(s), success, reward, latency)
                        }
                        Err(e) => (idx, name, call_id, args, Err(e), false, None, latency),
                    }
                }
            })
            .collect();

        let mut results = futures::future::join_all(tool_futures).await;
        results.sort_by_key(|(idx, ..)| *idx);

        // Process results sequentially for deterministic ordering
        for (_, tool_name, call_id, args, result, success, reward, latency_ms) in results {
            // Capture observation for the MCP-T behavior.trace emitter
            // regardless of success/failure — fidelity is about whether the
            // tool was in declared scope, not whether it succeeded.
            let _ = obs_tx
                .send(ToolCallObservation {
                    tool_name: tool_name.clone(),
                    timestamp: chrono::Utc::now(),
                    duration_ms: latency_ms,
                    declared: declared_scope.iter().any(|t| t == &tool_name),
                })
                .await;
            // SEQ-004: fold results into the session's taint sequentially, so
            // the accumulator sees a deterministic order and no batch contends
            // on its lock. Errors count too: one that echoes its argument
            // carries whatever was in it.
            #[cfg(feature = "taint")]
            if let Some(ref g) = egress {
                match &result {
                    Ok(body) => g.observe_result(&tool_name, &args, body),
                    Err(message) => g.observe_error(&tool_name, message),
                }
            }
            match result {
                Ok(result_str) => {
                    if let Some(r) = reward
                        && r < 0.0
                    {
                        info!("Executor: {} returned negative reward {:.3}", tool_name, r);
                    }

                    info!("Executor: {} succeeded ({}ms)", tool_name, latency_ms);

                    if let Some(mem) = tool_memory {
                        mem.write().await.add(tool_name.clone(), &args, &result_str);
                    }

                    if let Some(bus) = learning_bus {
                        let event = LearningEvent::ToolCall {
                            tool_name: tool_name.clone(),
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

                    if let Some(rt) = arkavo_arp_runtime::current() {
                        let quality = match reward {
                            Some(r) => f64::midpoint(r, 1.0).clamp(0.0, 1.0),
                            None if success => 1.0,
                            None => 0.0,
                        };
                        let ctx = arkavo_arp_runtime::ToolOutcomeContext::new()
                            .with_latency_ms(latency_ms);
                        rt.record_tool_outcome_with(&tool_name, success, quality, &ctx)
                            .await;
                    }

                    let _ = result_tx
                        .send(ExecutionResult {
                            tool_name,
                            call_id,
                            result: result_str,
                            success,
                            reward,
                        })
                        .await;
                }
                Err(err) => {
                    warn!("Executor: {} failed: {err}", tool_name);

                    if let Some(mem) = tool_memory {
                        mem.write()
                            .await
                            .add(tool_name.clone(), &args, &format!("Error: {err}"));
                    }

                    if let Some(rt) = arkavo_arp_runtime::current() {
                        let ctx = arkavo_arp_runtime::ToolOutcomeContext::new()
                            .with_latency_ms(latency_ms)
                            .with_error_type(err.clone());
                        rt.record_tool_outcome_with(&tool_name, false, 0.0, &ctx)
                            .await;
                    }

                    let _ = result_tx
                        .send(ExecutionResult {
                            tool_name,
                            call_id,
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

/// Judge track: condenses execution results and provides feedback to the planner.
/// Uses structural extraction (Delta sections, truncation) — no LLM calls.
/// On single GPU, LLM distillation contended with the planner and added 3-8s
/// per tool result to the feedback loop.
async fn judge_track(
    router: &Arc<arkavo_router::Router>,
    mut result_rx: mpsc::Receiver<ExecutionResult>,
    feedback_tx: mpsc::Sender<JudgeFeedback>,
) {
    let mut batch_results: Vec<CondensedToolResult> = Vec::new();
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

        // Condense results: always run through condense_tool_result for entity
        // extraction (names, alerts, rewards), even for small results.
        let distilled = if exec_result.result.len() > 1500 {
            // Structural condensation: extracts Delta/diff sections, truncates
            let condensed = super::conductor_tool_loop::condense_tool_result(
                &exec_result.tool_name,
                &exec_result.result,
                800,
            );

            // If structural condensation meaningfully reduced size, use it.
            // Otherwise fall back to LLM distillation for unstructured text.
            if condensed.len() < exec_result.result.len() / 2 {
                info!(
                    raw_len = exec_result.result.len(),
                    condensed_len = condensed.len(),
                    "Judge: condensed {} (structural)",
                    exec_result.tool_name
                );
                condensed
            } else {
                match distill_with_small_model(router, &exec_result.result).await {
                    Some(summary) => {
                        info!(
                            raw_len = exec_result.result.len(),
                            summary_len = summary.len(),
                            "Judge: distilled {} (LLM)",
                            exec_result.tool_name
                        );
                        summary
                    }
                    None => condensed, // structural is still better than nothing
                }
            }
        } else {
            // Small results: still run through condense for entity extraction preamble
            super::conductor_tool_loop::condense_tool_result(
                &exec_result.tool_name,
                &exec_result.result,
                800,
            )
        };

        let content = if distilled.len() > 800 {
            format!("{}...", &distilled[..800])
        } else {
            distilled
        };

        batch_results.push(CondensedToolResult {
            tool_name: exec_result.tool_name,
            call_id: exec_result.call_id,
            content,
        });

        // Send feedback after each result so the planner can proceed to
        // the next round with tool results in context. Previously batched
        // at 3 results, which deadlocked when the planner waited for
        // feedback before sending the next batch.
        {
            let feedback = JudgeFeedback {
                tool_results: std::mem::take(&mut batch_results),
                should_replan: has_negative_reward,
            };

            if feedback_tx.send(feedback).await.is_err() {
                break; // planner closed
            }

            has_negative_reward = false;
        }
    }

    // Flush remaining
    if !batch_results.is_empty() {
        let _ = feedback_tx
            .send(JudgeFeedback {
                tool_results: batch_results,
                should_replan: has_negative_reward,
            })
            .await;
    }

    info!("Judge: channel closed, shutting down");
}

#[cfg(test)]
mod tests {
    use arkavo_llm::ParsedToolCall;
    use serde_json::json;

    #[test]
    fn dedup_tool_calls_collapses_identical() {
        let calls: Vec<ParsedToolCall> = (0..10)
            .map(|_| ParsedToolCall {
                tool_name: "game-rl:step".to_string(),
                arguments: json!({"action": "move"}),
                call_id: None,
            })
            .collect();

        let deduped = super::dedup_tool_calls(calls);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn dedup_tool_calls_preserves_distinct() {
        let calls = vec![
            ParsedToolCall {
                tool_name: "game-rl:step".to_string(),
                arguments: json!({"action": "move"}),
                call_id: None,
            },
            ParsedToolCall {
                tool_name: "game-rl:observe".to_string(),
                arguments: json!({}),
                call_id: None,
            },
            ParsedToolCall {
                tool_name: "game-rl:step".to_string(),
                arguments: json!({"action": "build"}),
                call_id: None,
            },
        ];

        let deduped = super::dedup_tool_calls(calls);
        assert_eq!(deduped.len(), 3);
    }

    #[test]
    fn degenerate_batch_detection() {
        let small_batch: Vec<ParsedToolCall> = (0..3)
            .map(|_| ParsedToolCall {
                tool_name: "tool".to_string(),
                arguments: serde_json::Value::Object(serde_json::Map::new()),
                call_id: None,
            })
            .collect();
        assert!(!super::is_degenerate_batch(&small_batch));

        let large_batch: Vec<ParsedToolCall> = (0..25)
            .map(|_| ParsedToolCall {
                tool_name: "tool".to_string(),
                arguments: serde_json::Value::Object(serde_json::Map::new()),
                call_id: None,
            })
            .collect();
        assert!(super::is_degenerate_batch(&large_batch));
    }

    #[test]
    fn dedup_tool_calls_keeps_first_of_duplicates() {
        let calls = vec![
            ParsedToolCall {
                tool_name: "game-rl:step".to_string(),
                arguments: json!({"action": "move"}),
                call_id: Some("first".to_string()),
            },
            ParsedToolCall {
                tool_name: "game-rl:step".to_string(),
                arguments: json!({"action": "move"}),
                call_id: Some("second".to_string()),
            },
        ];

        let deduped = super::dedup_tool_calls(calls);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].call_id.as_deref(), Some("first"));
    }
}
