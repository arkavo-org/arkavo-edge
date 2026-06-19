use super::learning_bus::{LearningBus, LearningEvent};
use super::tool_memory::ToolMemory;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_router::BurstFeedback;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Single observed tool invocation captured during the loop. Used by the
/// MCP-T `behavior.trace` emitter to compute the fidelity ratio after the
/// task completes.
#[derive(Debug, Clone)]
pub(super) struct ToolCallObservation {
    pub tool_name: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    /// True if the tool was in the registry's declared scope at task start.
    pub declared: bool,
}

/// Result of running the agentic tool loop.
pub(super) struct ToolLoopResult {
    pub final_text: String,
    pub decision_model_name: Option<String>,
    pub total_latency_ms: u64,
    /// Peak context token count during the loop
    pub context_tokens: u32,
    /// Context window utilization (0-100%)
    pub context_utilization_pct: f64,
    /// LLM inference timing from the first iteration (populated by llama.cpp)
    pub inference_timing: Option<arkavo_llm::provider::InferenceTiming>,
    /// Total number of tool calls executed across all iterations
    pub tool_call_count: usize,
    /// Per-call observations collected during the loop, in execution order.
    /// Each carries a `declared` flag so the consumer can compute scope
    /// fidelity without a separate scope snapshot.
    pub tool_observations: Vec<ToolCallObservation>,
}

/// True if a tool call is permitted under an optional grant set.
/// `None` = unspecialized agent (no filtering). `Some(set)` = specialized;
/// only names in the set may execute.
pub(super) fn tool_call_permitted(
    name: &str,
    granted: Option<&std::collections::HashSet<String>>,
) -> bool {
    granted.is_none_or(|g| g.contains(name))
}

/// Run the agentic tool loop: LLM calls tools → results fed back → LLM continues.
/// Allows multi-step workflows like observe → delegate → monitor → execute.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_tool_loop(
    router: &Arc<arkavo_router::Router>,
    registry_arc: &Arc<ToolRegistry>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: &str,
    mut messages: Vec<arkavo_llm::Message>,
    model_hint: Option<&arkavo_router::ModelChoice>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<tokio::sync::RwLock<ToolMemory>>>,
    compute_budget: Option<&arkavo_budget::SharedComputeBudget>,
    granted_tools: Option<&std::collections::HashSet<String>>,
) -> Result<ToolLoopResult, String> {
    const MAX_TOOL_ITERATIONS: u8 = 4;
    let mut final_result = String::new();
    let mut reward_signals: Vec<f64> = Vec::new();
    let mut decision_trace_id = None;
    let mut decision_model_name = None;
    let mut total_step_idx: usize = 0;
    let mut first_inference_timing = None;
    let mut force_planning = false;
    let mut consecutive_negative_rewards: u32 = 0;
    let mut tool_observations: Vec<ToolCallObservation> = Vec::new();
    let declared_scope: Vec<String> = registry_arc
        .list_tools()
        .iter()
        .map(|t| t.name.clone())
        .collect();
    let loop_start = std::time::Instant::now();

    // Context budget: estimate model window in chars (tokens × 4)
    let model_ctx = super::rlm_bridge::model_context_size(
        model_hint.map(|h| h.name()),
        false, // conservative: assume local model
    );
    let char_budget = model_ctx * 4;

    for iteration in 0..MAX_TOOL_ITERATIONS {
        // Budget gate: auto-refresh if expired, stop if truly exhausted
        if let Some(budget) = compute_budget {
            let mut b = budget.write().await;
            if !b.has_remaining() {
                // Auto-refresh for autonomous agents whose TTL expired.
                // Specialists get refreshed by commander broadcasts;
                // autonomous agents (orchestrators, task-generators) need self-refresh.
                if b.remaining_inferences == 0 {
                    b.refresh(&arkavo_budget::BudgetAllocation {
                        max_inferences: 32,
                        max_tokens: 100_000,
                        ttl_secs: 600,
                        ..arkavo_budget::BudgetAllocation::default()
                    });
                    info!("Compute budget auto-refreshed at iteration {}", iteration);
                } else {
                    info!(
                        "Compute budget exhausted — stopping tool loop at iteration {}",
                        iteration
                    );
                    break;
                }
            }
        }

        // Execution iterations (1+) use a stripped inference profile:
        // temp 0.1, thinking off, max 200 tokens, 10s timeout.
        // Planning iterations (0) use full reasoning with model defaults.
        // Negative reward from previous iteration forces planning mode so the
        // capable model can reason about what went wrong and pick a better action.
        let is_execution = iteration > 0 && !force_planning;
        if force_planning {
            info!(
                iteration = iteration + 1,
                "Reward-driven escalation: using planning model"
            );
            force_planning = false;
        }

        // Smart context compaction: when messages exceed budget, distill old
        // messages into a compact summary using the fast model, then drop them.
        // Preserves key insights instead of losing context entirely.
        let mut context_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        if context_chars > char_budget && messages.len() > 3 {
            // Collect oldest non-system messages to compact (keep system + 2 recent)
            let keep_recent = 2;
            let compactable = messages.len() - 1 - keep_recent; // exclude system
            if compactable > 0 {
                let old_messages: Vec<String> = messages[1..=compactable]
                    .iter()
                    .map(|m| format!("[{:?}] {}", m.role, &m.content[..m.content.len().min(500)]))
                    .collect();
                let old_chars: usize = messages[1..=compactable]
                    .iter()
                    .map(|m| m.content.len())
                    .sum();

                // Distill via fast model — same pattern as tool result distillation
                let old_summary = old_messages.join("\n---\n");
                let summary = distill_with_small_model(router, &old_summary).await;

                // Remove the old messages
                for _ in 0..compactable {
                    messages.remove(1);
                }

                // Insert compact summary as context preamble (after system)
                if let Some(distilled) = summary {
                    info!(
                        old_msgs = compactable,
                        old_chars,
                        summary_chars = distilled.len(),
                        "Context compacted: distilled old messages via fast model"
                    );
                    messages.insert(
                        1,
                        arkavo_llm::Message::user(format!(
                            "[Previous context summary]: {distilled}"
                        )),
                    );
                } else {
                    // Fallback: structural summary when no fast model available
                    let structural = format!(
                        "[Previous context: {compactable} messages ({old_chars} chars) compacted]"
                    );
                    info!(
                        old_msgs = compactable,
                        old_chars, "Context compacted: structural fallback (no fast model)"
                    );
                    messages.insert(1, arkavo_llm::Message::user(structural));
                }

                context_chars = messages.iter().map(|m| m.content.len()).sum();
            }
        }

        // Context-aware timeout scaled by model generation speed.
        // Benchmarked TG speeds: 0.8B=170t/s, 3B=136t/s, 9B=50t/s, 27B=14t/s.
        // Formula: base covers ~500 token response + prompt eval overhead.
        let context_tokens = context_chars / 4;
        // Model-size-aware timeouts. Larger models are slower on all hardware;
        // the multiplier captures relative speed without hardcoding benchmarks.
        // Tier 1 (≤1B):  fast — base 30s exec, 90s plan
        // Tier 2 (1-5B):  mid — base 30s exec, 90s plan
        // Tier 3 (5-15B): slow — base 45s exec, 180s plan
        // Tier 4 (15B+):  very slow — base 60s exec, 240s plan
        let model_tier: u8 = match model_hint {
            Some(h) if h.size_bytes() >= 15_000_000_000 => 4,
            Some(h) if h.size_bytes() >= 5_000_000_000 => 3,
            Some(h) if h.size_bytes() >= 1_000_000_000 => 2,
            _ => 1,
        };
        let timeout_secs = if is_execution {
            // Scale base with model tier + context size.
            // Larger models need more time for prompt eval and generation.
            let base = match model_tier {
                4 => 60u64,
                3 => 45,
                _ => 30,
            };
            let context_extra = context_tokens as u64 / 500; // +1s per 500 tokens
            (base + context_extra).clamp(30, 120)
        } else {
            let (base_timeout, max_timeout) = match model_tier {
                4 => (240u64, 360u64),
                3 => (180u64, 300u64),
                _ => (90u64, 180u64),
            };
            if context_tokens <= 2000 {
                base_timeout
            } else {
                let extra = ((context_tokens - 2000) / 500) as u64;
                (base_timeout + extra).min(max_timeout)
            }
        };

        let msg_count = messages.len();
        let largest_msg = messages.iter().map(|m| m.content.len()).max().unwrap_or(0);
        info!(
            iteration = iteration + 1,
            is_execution,
            msg_count,
            context_chars,
            context_tokens,
            largest_msg_chars = largest_msg,
            timeout_secs,
            model = model_hint.map(|h| h.name()).unwrap_or("auto"),
            "Tool loop: starting inference"
        );

        let inference_start = std::time::Instant::now();
        let timeout_dur = std::time::Duration::from_secs(timeout_secs);
        let response = if is_execution {
            tokio::time::timeout(
                timeout_dur,
                router.route_with_tools_execution(
                    task_content,
                    messages.clone(),
                    Some(registry_arc),
                    model_hint,
                ),
            )
            .await
        } else {
            tokio::time::timeout(
                timeout_dur,
                router.route_with_tools_hinted(
                    task_content,
                    messages.clone(),
                    Some(registry_arc),
                    model_hint,
                ),
            )
            .await
        };
        let response = match response {
            Ok(inner) => {
                let inference_ms = inference_start.elapsed().as_millis();
                info!(
                    iteration = iteration + 1,
                    is_execution,
                    inference_ms,
                    context_tokens,
                    timeout_secs,
                    "Tool loop: inference completed"
                );
                // Consume one inference from compute budget so specialists
                // respect their per-window allocation and don't saturate the GPU.
                if let Some(budget) = compute_budget {
                    let mut b = budget.write().await;
                    b.consume_inference(context_tokens as u64, 0.0);
                }
                match inner {
                    Ok(resp) => {
                        if resp.quality_gate_retries > 0 {
                            info!(
                                iteration = iteration + 1,
                                retries = resp.quality_gate_retries,
                                "Quality gate retries before success"
                            );
                        }
                        resp
                    }
                    Err(e) => {
                        if e.is_gpu_fault() {
                            warn!(
                                iteration = iteration + 1,
                                error = %e,
                                "GPU fault during inference — recording negative feedback"
                            );
                            let faulted_model = router
                                .last_routed_model()
                                .or_else(|| model_hint.map(|h| h.name().to_string()));
                            if let Some(model_name) = faulted_model {
                                let feedback = BurstFeedback::failure(
                                    uuid::Uuid::new_v4(),
                                    "gpu_fault".to_string(),
                                    0,
                                )
                                .with_quality(0.3); // operational, not quality
                                router
                                    .model_learning()
                                    .immediate_update(&model_name, &feedback)
                                    .await;
                                router.record_quality_cooldown(&model_name).await;
                            }
                        } else {
                            warn!(
                                iteration = iteration + 1,
                                error = %e,
                                "Tool loop inference error — breaking loop"
                            );
                        }
                        break;
                    }
                }
            }
            Err(_elapsed) => {
                warn!(
                    iteration = iteration + 1,
                    timeout_secs, context_tokens, "Tool loop inference timed out — breaking loop"
                );
                let timed_out_model = router
                    .last_routed_model()
                    .or_else(|| model_hint.map(|h| h.name().to_string()));
                if let Some(model_name) = timed_out_model {
                    // Operational failure: mild penalty, not full quality rejection
                    let feedback = BurstFeedback::failure(
                        uuid::Uuid::new_v4(),
                        "timeout".to_string(),
                        timeout_secs * 1000,
                    )
                    .with_quality(0.3);
                    router
                        .model_learning()
                        .immediate_update(&model_name, &feedback)
                        .await;
                    // Execution timeouts (30s cap) are expected under load —
                    // don't cooldown the model, just record feedback
                    if !is_execution {
                        router.record_quality_cooldown(&model_name).await;
                    }
                    info!(
                        model = %model_name,
                        is_execution,
                        "Timeout: recorded negative feedback"
                    );
                }
                if let Some(hint) = model_hint {
                    router.advisor().observe(hint.family(), task_content, "");
                }
                break;
            }
        };

        if iteration == 0 {
            let trace = router.last_decision_trace();
            decision_trace_id = trace.as_ref().map(|t| t.trace_id);
            decision_model_name = router.last_routed_model();
            first_inference_timing.clone_from(&response.inference_timing);
        }

        // Feed spec-decoding outcome back to the router's per-model rolling stats.
        // Only record when spec actually ran — bypassed paths (grammar, stops, etc.)
        // give no quality signal for this model's spec compatibility.
        if let Some(ref timing) = response.inference_timing
            && timing.spec_bypassed.is_none()
            && let (Some(n_d), Some(n_a)) = (timing.n_draft, timing.n_accepted)
        {
            let model_name = router
                .last_routed_model()
                .or_else(|| model_hint.map(|h| h.name().to_string()));
            if let Some(model) = model_name {
                router.spec_stats().record(&model, n_d, n_a);
            }
        }

        info!(
            "Tool loop iteration {}: {} chars, {} tool calls",
            iteration + 1,
            response.content.len(),
            response.tool_calls.len()
        );

        if std::env::var("ARKAVO_DEBUG").is_ok() {
            eprintln!(
                "[LLM Response iter {}] {} chars:",
                iteration + 1,
                response.content.len()
            );
            eprintln!(
                "{}",
                &response.content[..std::cmp::min(1000, response.content.len())]
            );
        }

        debug!(
            "LLM response content: {}",
            if response.content.len() > 500 {
                format!("{}...", &response.content[..500])
            } else {
                response.content.clone()
            }
        );

        if !response.content.is_empty() {
            if !final_result.is_empty() {
                final_result.push('\n');
            }
            final_result.push_str(&response.content);
        }

        if response.tool_calls.is_empty() {
            // Agents without MCP tools (specialists/advisors) produce text-only
            // output by design — don't penalize them for not calling tools.
            let has_mcp_tools = mcp_registry
                .list_all_tools()
                .await
                .map(|t| !t.is_empty())
                .unwrap_or(false);
            if iteration == 0
                && has_mcp_tools
                && let Some(model_name) = router.last_routed_model()
            {
                warn!("LLM did not request any tool calls — penalizing model");
                let feedback =
                    BurstFeedback::failure(uuid::Uuid::new_v4(), "no_tool_call".to_string(), 0)
                        .with_quality(0.0);
                router
                    .model_learning()
                    .immediate_update(&model_name, &feedback)
                    .await;
                router.record_quality_cooldown(&model_name).await;
                let family = arkavo_router::Router::detect_model_family(&model_name);
                router
                    .advisor()
                    .observe_no_tool_calls(&family, &response.content);
                info!(model = %model_name, "No tool calls: recorded negative feedback and cooldown");
            }
            break;
        }

        let tool_count = response.tool_calls.len();
        info!("Executing {tool_count} tool calls (step {})", iteration + 1);

        // Record the calls the assistant issued, with their real arguments, so
        // chat templates (Gemma 4) render a faithful call block instead of
        // reconstructing one with empty arguments from the tool results. Calls
        // are emitted in the same order as the tool results pushed below, so the
        // template pairs them positionally and by call id.
        let assistant_tool_calls: Vec<arkavo_llm::ToolCall> = response
            .tool_calls
            .iter()
            .map(|tc| arkavo_llm::ToolCall {
                name: tc.tool_name.clone(),
                arguments: tc.arguments.to_string(),
                id: tc.call_id.clone(),
            })
            .collect();
        messages.push(arkavo_llm::Message::assistant_with_tool_calls(
            response.content.clone(),
            assistant_tool_calls,
        ));

        let tool_result_parts = execute_tool_calls(
            &response.tool_calls,
            registry_arc,
            mcp_registry,
            router,
            learning_bus,
            tool_memory,
            decision_trace_id,
            decision_model_name.as_ref(),
            &mut reward_signals,
            &mut total_step_idx,
            &declared_scope,
            &mut tool_observations,
            granted_tools,
        )
        .await;

        // Negative reward from game actions → escalate next iteration to planning mode
        // so the capable model can reason about the failure and pick a better action.
        if reward_signals.last().is_some_and(|&r| r < 0.0) {
            force_planning = true;
            consecutive_negative_rewards += 1;

            // Sustained failure → synthesize a recovery lesson via learning bus.
            // At 10+ consecutive negatives the environment is in systemic failure;
            // teach the agent to assess performance and consider recovery actions.
            if consecutive_negative_rewards == 10
                && let Some(bus) = learning_bus
            {
                bus.add_fast_lesson(
                    "sustained_failure",
                    "10+ consecutive negative rewards indicate systemic failure. \
                     Assess cumulative reward via summary/status tools. \
                     If reward is very negative, consider resetting or loading a checkpoint.",
                )
                .await;
            }
        } else if reward_signals.last().is_some() {
            consecutive_negative_rewards = 0;
        }

        let tool_results_text = tool_result_parts.join("\n\n");
        let raw_result_chars = tool_results_text.len();

        // Large tool results (status dumps, state queries) get distilled by a
        // small fast model so the main model receives only noteworthy items.
        // Skip distillation when the result fits comfortably in the context
        // budget — the 10s timeout adds unacceptable latency to game loops.
        let current_context_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let projected_with_result = current_context_chars + tool_results_text.len();
        let needs_distillation = tool_results_text.len() > SMALL_MODEL_SUMMARIZE_THRESHOLD
            && projected_with_result > char_budget * 40 / 100;
        let result_to_append = if needs_distillation {
            match distill_with_small_model(router, &tool_results_text).await {
                Some(summary) => {
                    info!(
                        raw_chars = raw_result_chars,
                        distilled_chars = summary.len(),
                        "Tool results distilled by small model"
                    );
                    summary
                }
                None => {
                    // Fallback to structural summarization
                    let summarized = if projected_with_result > char_budget * 80 / 100 {
                        summarize_tool_results(&tool_result_parts, 500)
                    } else if projected_with_result > char_budget * 60 / 100 {
                        summarize_tool_results(&tool_result_parts, 1500)
                    } else {
                        tool_results_text
                    };
                    info!(
                        raw_chars = raw_result_chars,
                        summarized_chars = summarized.len(),
                        projected_context = projected_with_result,
                        char_budget,
                        "Tool results: distillation failed, structural fallback"
                    );
                    summarized
                }
            }
        } else {
            tool_results_text
        };

        let exploration_nudge = if consecutive_negative_rewards >= 3 {
            info!(
                consecutive_negative_rewards,
                "Injecting exploration prompt after repeated negative rewards"
            );
            format!(
                "\n\nWARNING: The last {consecutive_negative_rewards} actions produced negative rewards. \
                 Choose a different action category than the last 3 ticks. \
                 Observe the current state for the most critical unmet need \
                 and address it directly."
            )
        } else {
            String::new()
        };

        // Push tool results as proper tool-role messages so Jinja templates
        // (especially Gemma 4) generate the correct token structure.
        // Falls back to user-role for models without native tool response support.
        if response.tool_calls.len() == 1 {
            let tc = &response.tool_calls[0];
            let call_id = tc
                .call_id
                .clone()
                .unwrap_or_else(|| format!("call_{total_step_idx}"));
            messages.push(arkavo_llm::Message::tool_result(
                format!("{result_to_append}{exploration_nudge}"),
                call_id,
                &tc.tool_name,
            ));
        } else {
            // Multiple tool calls: push one tool_result per call
            for (i, tc) in response.tool_calls.iter().enumerate() {
                let call_id = tc
                    .call_id
                    .clone()
                    .unwrap_or_else(|| format!("call_{total_step_idx}_{i}"));
                let part = tool_result_parts
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| "ok".to_string());
                messages.push(arkavo_llm::Message::tool_result(
                    part,
                    call_id,
                    &tc.tool_name,
                ));
            }
            if !exploration_nudge.is_empty() {
                messages.push(arkavo_llm::Message::user(exploration_nudge));
            }
        }
    }

    // Synthesize a minimal summary when the LLM's last turn was a tool call
    // (not a text response). Without this, compute_response_quality("", ...) returns
    // 0.0, keeping Thompson Sampling avg_quality stuck at 0%.
    if final_result.is_empty() && total_step_idx > 0 {
        final_result = format!(
            "Completed {} tool call(s). Last result: {}",
            total_step_idx,
            messages
                .last()
                .map(|m| &m.content[..m.content.len().min(200)])
                .unwrap_or("ok")
        );
    }

    apply_reward_correction(router, &reward_signals).await;

    // Compute context utilization
    let peak_context_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    let peak_context_tokens = (peak_context_chars / 4) as u32;
    let context_utilization_pct = (peak_context_tokens as f64 / model_ctx as f64) * 100.0;

    let total_latency_ms = loop_start.elapsed().as_millis() as u64;

    // Record conductor orchestration latency to global subsystem timing
    arkavo_observability::subsystem_timing::global_timing()
        .conductor_orchestration
        .record(total_latency_ms);

    Ok(ToolLoopResult {
        final_text: final_result,
        decision_model_name,
        total_latency_ms,
        context_tokens: peak_context_tokens,
        context_utilization_pct,
        inference_timing: first_inference_timing,
        tool_call_count: total_step_idx,
        tool_observations,
    })
}

#[allow(clippy::too_many_arguments)]
async fn execute_tool_calls(
    tool_calls: &[arkavo_llm::ParsedToolCall],
    registry_arc: &Arc<ToolRegistry>,
    mcp_registry: &Arc<McpRegistry>,
    router: &Arc<arkavo_router::Router>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<tokio::sync::RwLock<ToolMemory>>>,
    decision_trace_id: Option<uuid::Uuid>,
    decision_model_name: Option<&String>,
    reward_signals: &mut Vec<f64>,
    total_step_idx: &mut usize,
    declared_scope: &[String],
    observations: &mut Vec<ToolCallObservation>,
    granted_tools: Option<&std::collections::HashSet<String>>,
) -> Vec<String> {
    let mut tool_result_parts = Vec::new();

    for tool_call in tool_calls {
        // Enforce least-privilege: deny tool calls not in the agent's grant set.
        if !tool_call_permitted(&tool_call.tool_name, granted_tools) {
            warn!(
                tool = %tool_call.tool_name,
                "Tool call denied: not in this agent's granted set (least-privilege)"
            );
            tool_result_parts.push(format!(
                "Tool {} (Denied): not permitted for this role — \
                 it is not in the agent's granted tool set.",
                tool_call.tool_name
            ));
            *total_step_idx += 1;
            continue;
        }

        // Skip setup tools that already succeeded (e.g., registerAgent)
        if let Some(mem) = tool_memory
            && mem.read().await.is_setup_complete(&tool_call.tool_name)
        {
            info!(
                "Skipping {} (already completed setup tool)",
                tool_call.tool_name
            );
            tool_result_parts.push(format!(
                "{}: Already completed — skipped",
                tool_call.tool_name
            ));
            continue;
        }

        let args = tool_call.arguments.clone();
        debug!(
            "Tool call: {} with args: {}",
            tool_call.tool_name,
            serde_json::to_string(&args).unwrap_or_default()
        );

        let start_time = std::time::Instant::now();

        let tool_result = if let Some(tool) = registry_arc.get(&tool_call.tool_name) {
            tool.execute(args.clone()).await.map_err(|e| e.to_string())
        } else {
            mcp_registry
                .call_tool(&tool_call.tool_name, args.clone(), "hrm")
                .await
                .map_err(|e| e.to_string())
        };

        match tool_result {
            Ok(result) => {
                let latency_ms = start_time.elapsed().as_millis() as u64;
                arkavo_observability::subsystem_timing::global_timing()
                    .mcp_tools
                    .record(latency_ms);
                observations.push(ToolCallObservation {
                    tool_name: tool_call.tool_name.clone(),
                    timestamp: chrono::Utc::now(),
                    duration_ms: latency_ms,
                    declared: declared_scope.iter().any(|t| t == &tool_call.tool_name),
                });
                let result_str = serde_json::to_string(&result).unwrap_or_default();

                let reward = super::conductor::extract_reward_from_result(&result_str);
                let semantic_failure = detect_semantic_failure(&result_str);
                let tool_success = reward.is_none_or(|r| r >= 0.0) && semantic_failure.is_none();

                if let Some(r) = reward {
                    reward_signals.push(r);
                    if r < 0.0 {
                        info!(
                            "Tool {} returned negative reward {:.3}",
                            tool_call.tool_name, r
                        );
                    }
                } else if let Some(ref err_msg) = semantic_failure {
                    warn!("Tool {} action failed: {}", tool_call.tool_name, err_msg);
                    // Record anti-pattern for repeated semantic failures
                    if let Some(model_name) = router.last_routed_model() {
                        let model_family = arkavo_router::Router::detect_model_family(&model_name);
                        router.advisor().observe_tool_error(
                            &model_family,
                            &tool_call.tool_name,
                            err_msg,
                            &args,
                        );
                        if let Some(bus) = learning_bus {
                            use super::anti_pattern::AntiPatternStore;
                            let signature =
                                AntiPatternStore::classify_failure(&tool_call.tool_name, err_msg);
                            bus.record_human_correction(
                                &signature,
                                decision_trace_id,
                                Some(&model_name),
                            )
                            .await;

                            // Fast-path lesson: tool error messages are ground truth.
                            // Bypass the 3-observation accumulation threshold and write
                            // a corrective lesson directly to PolicyCache.
                            bus.add_fast_lesson(&tool_call.tool_name, err_msg).await;
                        }
                    }
                } else {
                    info!("Tool {} succeeded", tool_call.tool_name);
                }
                debug!("Tool {} result: {}", tool_call.tool_name, result_str);

                if let Some(mem) = tool_memory {
                    mem.write()
                        .await
                        .add(tool_call.tool_name.clone(), &args, &result_str);
                }

                if let Some(bus) = learning_bus {
                    let event = LearningEvent::ToolCall {
                        tool_name: tool_call.tool_name.clone(),
                        args: args.clone(),
                        result: result_str.clone(),
                        success: tool_success,
                        latency_ms,
                        decision_trace_id,
                        step_index: *total_step_idx as u16,
                        model_name: decision_model_name.cloned(),
                    };
                    let _ = bus.sender().send(event).await;
                }

                // ARP runtime: feed tool outcome into the per-agent
                // PolicyCache, AdaptationEngine, and DecisionTrace so they
                // evaluate on every step (ARP §6, §7.2, §17.1).
                if let Some(rt) = arkavo_arp_runtime::current() {
                    let quality = match reward {
                        Some(r) => f64::midpoint(r, 1.0).clamp(0.0, 1.0),
                        None if tool_success => 1.0,
                        None => 0.0,
                    };
                    let ctx =
                        arkavo_arp_runtime::ToolOutcomeContext::new().with_latency_ms(latency_ms);
                    rt.record_tool_outcome_with(&tool_call.tool_name, tool_success, quality, &ctx)
                        .await;
                }

                tool_result_parts.push(condense_tool_result(
                    &tool_call.tool_name,
                    &result_str,
                    4000,
                ));
            }
            Err(err_str) => {
                let latency_ms = start_time.elapsed().as_millis() as u64;
                arkavo_observability::subsystem_timing::global_timing()
                    .mcp_tools
                    .record(latency_ms);
                observations.push(ToolCallObservation {
                    tool_name: tool_call.tool_name.clone(),
                    timestamp: chrono::Utc::now(),
                    duration_ms: latency_ms,
                    declared: declared_scope.iter().any(|t| t == &tool_call.tool_name),
                });
                warn!("Tool {} failed: {}", tool_call.tool_name, err_str);

                if let Some(mem) = tool_memory {
                    mem.write().await.add(
                        tool_call.tool_name.clone(),
                        &args,
                        &format!("Error: {err_str}"),
                    );
                }

                if let Some(bus) = learning_bus {
                    let event = LearningEvent::ToolCall {
                        tool_name: tool_call.tool_name.clone(),
                        args: args.clone(),
                        result: format!("Error: {err_str}"),
                        success: false,
                        latency_ms,
                        decision_trace_id,
                        step_index: *total_step_idx as u16,
                        model_name: decision_model_name.cloned(),
                    };
                    let _ = bus.sender().send(event).await;
                }

                if let Some(model_name) = router.last_routed_model() {
                    let model_family = arkavo_router::Router::detect_model_family(&model_name);
                    router.advisor().observe_tool_error(
                        &model_family,
                        &tool_call.tool_name,
                        &err_str,
                        &args,
                    );

                    if let Some(bus) = learning_bus {
                        use super::anti_pattern::AntiPatternStore;
                        let signature =
                            AntiPatternStore::classify_failure(&tool_call.tool_name, &err_str);
                        bus.record_human_correction(
                            &signature,
                            decision_trace_id,
                            Some(&model_name),
                        )
                        .await;

                        // Fast-path lesson for transport-level errors too.
                        // Without this, schema violations (e.g. missing "Type"
                        // field) are lost between agent loop cycles because the
                        // conversation is cleared.
                        bus.add_fast_lesson(&tool_call.tool_name, &err_str).await;
                    }
                }

                // ARP runtime: record the failure so the AdaptationEngine
                // prior reflects the bad outcome (§6), the cache logs it,
                // and a `tool_invocation` trace row captures the error.
                if let Some(rt) = arkavo_arp_runtime::current() {
                    let ctx = arkavo_arp_runtime::ToolOutcomeContext::new()
                        .with_latency_ms(latency_ms)
                        .with_error_type(err_str.clone());
                    rt.record_tool_outcome_with(&tool_call.tool_name, false, 0.0, &ctx)
                        .await;
                }

                tool_result_parts
                    .push(format!("Tool {} (Error): {}", tool_call.tool_name, err_str));
            }
        }
        *total_step_idx += 1;
    }

    tool_result_parts
}

/// Tool results above this size are distilled by a small model before being
/// fed back to the main model. Keeps context lean for expensive inference.
const SMALL_MODEL_SUMMARIZE_THRESHOLD: usize = 1500;

/// Use the smallest available local model to distill large tool results into
/// a brief summary of noteworthy items. Returns `None` if no small model is
/// available, letting the caller fall back to structural summarization.
///
/// Uses `router.route_fast()` which acquires the synthesis semaphore,
/// serializing with other inference calls to avoid GPU/KV-cache contention.
pub(super) async fn distill_with_small_model(
    router: &Arc<arkavo_router::Router>,
    raw_results: &str,
) -> Option<String> {
    // Cap input to the small model — it has a limited context window
    let truncated: String = raw_results.chars().take(3000).collect();

    let messages = vec![
        arkavo_llm::Message::system(
            "You are a concise status analyst. Given tool output, list ONLY \
             noteworthy or changed items. Omit routine/unchanged data. \
             Keep your response under 300 words.",
        ),
        arkavo_llm::Message::user(truncated),
    ];

    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        router.route_fast("distill tool results", messages),
    )
    .await
    {
        Ok(Ok(stream)) => match stream.complete().await {
            Ok(response) if !response.content.is_empty() => {
                info!(
                    raw_len = raw_results.len(),
                    summary_len = response.content.len(),
                    "Small model distilled tool results"
                );
                Some(response.content)
            }
            Ok(_) => {
                debug!("Small model distillation returned empty");
                None
            }
            Err(e) => {
                debug!("Small model distillation stream failed: {e}");
                None
            }
        },
        Ok(Err(e)) => {
            debug!("Small model distillation failed: {e}");
            None
        }
        _ => {
            debug!("Small model distillation timed out");
            None
        }
    }
}

/// Condense a single tool result to fit within a character budget.
///
/// First extracts actionable entities (colonist names, IDs, alerts) from the
/// full result and prepends them as a structured preamble. Then extracts the
/// `"Delta"` section for state changes. The model gets entity names it can
/// use directly in subsequent tool calls without parsing raw JSON.
pub(super) fn condense_tool_result(tool_name: &str, raw: &str, max_chars: usize) -> String {
    let prefix = format!("Tool {tool_name}: ");
    let budget = max_chars.saturating_sub(prefix.len());

    // Strip large embedded schemas (ActionSpace, ObservationSpace, etc.) that
    // blow up context windows. Replace with a count summary so the model knows
    // the schema exists without carrying 5KB of JSON per conversation turn.
    let raw = strip_embedded_schemas(raw);

    // Always extract actionable entities — even small results may contain
    // entity names in error messages that the model needs for next calls
    let preamble = extract_entities(&raw);

    if raw.len() <= budget {
        if preamble.is_empty() {
            return format!("{prefix}{raw}");
        }
        return format!("{prefix}{preamble}\n{raw}");
    }

    // Extract Delta from the FULL raw result before any truncation
    let delta_section = if let Some(delta_start) = raw.find("\"Delta\":{") {
        let subset = &raw[delta_start..];
        find_matching_brace(subset).map(|end| format!("{{{}}}", &subset[..=end]))
    } else {
        None
    };

    // Build result: preamble + delta (or truncated raw)
    let content_budget = budget.saturating_sub(preamble.len());
    let body = if let Some(ref delta) = delta_section {
        if delta.len() <= content_budget {
            delta.clone()
        } else {
            format!(
                "{}...(truncated)",
                &raw[..content_budget.saturating_sub(20).min(raw.len())]
            )
        }
    } else {
        format!(
            "{}...(truncated {} total chars)",
            &raw[..content_budget.saturating_sub(40).min(raw.len())],
            raw.len()
        )
    };

    if preamble.is_empty() {
        format!("{prefix}{body}")
    } else {
        format!("{prefix}{preamble}\n{body}")
    }
}

/// Strip large embedded schemas from tool results to prevent context blowup.
///
/// Registration responses often include full ActionSpace/ObservationSpace schemas
/// (5KB+) that are useful for reference but shouldn't be in every conversation
/// turn. Replaces them with a count summary.
fn strip_embedded_schemas(raw: &str) -> String {
    // Try to parse as JSON to find schema-like objects
    let Ok(mut json) = serde_json::from_str::<serde_json::Value>(raw) else {
        // If the raw string contains escaped JSON (double-encoded), try that
        let unescaped = raw.replace("\\\"", "\"").replace("\\\\", "\\");
        let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&unescaped) else {
            return raw.to_string();
        };
        return strip_schemas_from_value(&mut json);
    };
    strip_schemas_from_value(&mut json)
}

fn strip_schemas_from_value(json: &mut serde_json::Value) -> String {
    // Also handle double-encoded "result" field (common in MCP bridge responses)
    if let Some(result_str) = json.get("result").and_then(|v| v.as_str())
        && let Ok(mut inner) = serde_json::from_str::<serde_json::Value>(result_str)
    {
        let changed = strip_schema_fields(&mut inner);
        if changed {
            json["result"] = serde_json::Value::String(inner.to_string());
        }
    }
    strip_schema_fields(json);
    json.to_string()
}

/// Replace large schema-shaped fields with count summaries.
///
/// A field is schema-shaped if it's a large array of objects where each
/// element contains "type" or "description" subfields (JSON Schema pattern).
/// This catches tool/action/parameter definitions without hardcoding
/// domain-specific key names.
/// Returns true if any modifications were made.
fn strip_schema_fields(json: &mut serde_json::Value) -> bool {
    let mut changed = false;
    if let Some(obj) = json.as_object_mut() {
        let schema_keys: Vec<String> = obj
            .iter()
            .filter(|(_, v)| is_schema_shaped(v))
            .map(|(k, _)| k.clone())
            .collect();

        for key in &schema_keys {
            if let Some(val) = obj.get(key) {
                let summary = match val {
                    serde_json::Value::Array(a) => format!("[{} items]", a.len()),
                    serde_json::Value::Object(o) => format!("{{{} fields}}", o.len()),
                    _ => continue,
                };
                obj.insert(key.clone(), serde_json::Value::String(summary));
                changed = true;
            }
        }

        // Recurse into remaining object values
        for val in obj.values_mut() {
            if strip_schema_fields(val) {
                changed = true;
            }
        }
    }
    changed
}

/// Returns true if a value looks like a schema definition rather than runtime data.
///
/// Detects arrays of objects with "type"/"description" fields (JSON Schema pattern)
/// and objects whose children are mostly schema-shaped (like an ObservationSpace).
fn is_schema_shaped(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Array(a) if a.len() > 5 => {
            // Array of objects where most have "Type"/"type" + "Description"/"description"
            let schema_count = a
                .iter()
                .filter(|item| {
                    item.get("type").or_else(|| item.get("Type")).is_some()
                        && item
                            .get("description")
                            .or_else(|| item.get("Description"))
                            .is_some()
                })
                .count();
            schema_count > a.len() / 2
        }
        serde_json::Value::Object(o) if o.len() > 5 => {
            // Object where most children have a "type" subfield (schema definition)
            let schema_count = o
                .values()
                .filter(|child| child.get("type").is_some() && child.as_object().is_some())
                .count();
            schema_count > o.len() / 2
        }
        _ => false,
    }
}

/// Extract actionable entities from a tool result.
/// Uses generic patterns that work across any MCP server:
/// - Named entity lists in brackets: `['A', 'B', 'C']`
/// - Alert/label fields with severity
/// - Reward signals
fn extract_entities(raw: &str) -> String {
    let mut parts = Vec::new();

    // Extract entity names from bracketed lists.
    // Handles both raw JSON (['A', 'B']) and escaped JSON ([\'A\', \'B\'])
    // which occurs in double-encoded MCP results.
    let unescaped;
    let search_str = if raw.contains("\\'") || raw.contains("\\\"") {
        unescaped = raw.replace("\\'", "'").replace("\\\"", "\"");
        &unescaped
    } else {
        raw
    };
    if let Some(start) = search_str.find("['")
        && let Some(end) = search_str[start..].find(']')
    {
        let names_str = &search_str[start + 1..start + end];
        let names: Vec<&str> = names_str
            .split(',')
            .map(|s| s.trim().trim_matches('\'').trim_matches('"').trim())
            .filter(|s| !s.is_empty() && s.len() > 1)
            .collect();
        if !names.is_empty() {
            parts.push(format!("Available entities: {}", names.join(", ")));
        }
    }

    // Extract alerts
    let mut alerts = Vec::new();
    let mut search = raw;
    while let Some(pos) = search.find("\"Label\":\"") {
        let label_start = pos + 9;
        if let Some(label_end) = search[label_start..].find('"') {
            let label = &search[label_start..label_start + label_end];
            // Get severity if nearby
            let nearby = &search[pos..search.len().min(pos + 100)];
            let severity = if let Some(s) = nearby.find("\"Severity\":") {
                nearby[s + 11..].chars().next().and_then(|c| c.to_digit(10))
            } else {
                None
            };
            let alert = match severity {
                Some(s) => format!("{label} (sev {s})"),
                None => label.to_string(),
            };
            if !alerts.contains(&alert) {
                alerts.push(alert);
            }
            search = &search[label_start + label_end..];
        } else {
            break;
        }
    }
    if !alerts.is_empty() {
        parts.push(format!("Alerts: {}", alerts.join(", ")));
    }

    // Extract reward
    if let Some(pos) = raw.find("\"Reward\":") {
        let num_start = pos + 9;
        let num_str: String = raw[num_start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        if let Ok(reward) = num_str.parse::<f64>() {
            parts.push(format!("Reward: {reward:.2}"));
        }
    }

    parts.join(" | ")
}

/// Summarize tool results to fit within a per-tool character budget.
///
/// Tries to extract just the `"Delta"` section from game state JSON, which
/// contains only what changed since the last observation. Falls back to
/// simple truncation when no Delta is found.
fn summarize_tool_results(parts: &[String], max_chars_per_tool: usize) -> String {
    parts
        .iter()
        .map(|part| {
            if part.len() <= max_chars_per_tool {
                return part.clone();
            }
            // Try to extract just the Delta section from game state JSON
            if let Some(delta_start) = part.find("\"Delta\":{") {
                let subset = &part[delta_start..];
                if let Some(end) = find_matching_brace(subset) {
                    let delta = &subset[..=end];
                    if delta.len() <= max_chars_per_tool {
                        let prefix = part.split(':').next().unwrap_or("Tool");
                        return format!("{prefix}: {{{delta}}}");
                    }
                }
            }
            // Fallback: truncate
            format!("{}...(summarized)", &part[..max_chars_per_tool])
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Find the index of the closing brace that matches the first `{` in `s`.
pub(super) fn find_matching_brace(s: &str) -> Option<usize> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, ch) in s[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

async fn apply_reward_correction(router: &Arc<arkavo_router::Router>, reward_signals: &[f64]) {
    if reward_signals.is_empty() {
        return;
    }
    let Some(model_name) = router.last_routed_model() else {
        return;
    };

    let avg_reward = reward_signals.iter().sum::<f64>() / reward_signals.len() as f64;
    let quality = f64::midpoint(avg_reward, 1.0).clamp(0.0, 1.0);
    let feedback = if avg_reward >= 0.0 {
        BurstFeedback::success(uuid::Uuid::new_v4(), "reward_correction".to_string(), 0)
            .with_quality(quality)
    } else {
        BurstFeedback::failure(uuid::Uuid::new_v4(), "reward_correction".to_string(), 0)
            .with_quality(quality)
    };
    info!(
        model = %model_name,
        avg_reward = format!("{avg_reward:.3}").as_str(),
        quality = format!("{quality:.3}").as_str(),
        "Reward correction applied to Thompson Sampling"
    );
    router
        .model_learning()
        .immediate_update(&model_name, &feedback)
        .await;

    // Track sustained negative rewards separately from cooldowns so the
    // model hint can be released after HINT_OVERRIDE_THRESHOLD consecutive
    // negative-reward ticks (availability cooldowns are cleared on success).
    if avg_reward < -0.3 {
        router.record_reward_failure(&model_name).await;
    } else if avg_reward > 0.0 {
        router.clear_reward_failure(&model_name).await;
    }
}

/// Detect semantic failures in tool results that arrive as successful RPC
/// responses but indicate the action failed at the application level.
///
/// Searches the result JSON tree for any object containing `"Success": false`
/// (case-insensitive key) paired with a `"Message"` field. Works with
/// double-wrapped JSON (`{"result": "..."}`) and arbitrarily nested structures.
fn detect_semantic_failure(result_json: &str) -> Option<String> {
    let outer: serde_json::Value = serde_json::from_str(result_json).ok()?;

    // Unwrap the MCP double-wrapping: {"result": "{...}"}
    let inner = if let Some(inner_str) = outer.get("result").and_then(|v| v.as_str()) {
        serde_json::from_str(inner_str).unwrap_or(outer)
    } else {
        outer
    };

    find_failure_in_value(&inner)
}

/// Recursively search a JSON value for `"Success": false` with a sibling `"Message"`.
fn find_failure_in_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            // Check if this object has Success:false
            let has_failure = map.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("success") && v == &serde_json::Value::Bool(false)
            });
            if has_failure {
                let msg = map
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("message"))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("Action failed");
                return Some(msg.to_string());
            }
            // Recurse into child values
            for v in map.values() {
                if let Some(msg) = find_failure_in_value(v) {
                    return Some(msg);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(msg) = find_failure_in_value(v) {
                    return Some(msg);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-009")]
    #[test]
    fn test_summarize_short_results_unchanged() {
        let parts = vec!["Tool sim_step: {\"ok\":true}".to_string()];
        let result = summarize_tool_results(&parts, 500);
        assert_eq!(result, "Tool sim_step: {\"ok\":true}");
    }

    #[spec("SRV-009")]
    #[test]
    fn test_summarize_extracts_delta() {
        let game_state = format!(
            "Tool sim_step: {{\"Observation\":{{\"Tick\":100,\"Delta\":{{\"Resources\":{{\"wood\":50}}}},\"Research\":{{{}}}}}}}",
            "\"a\":1,".repeat(200) // pad to make it large
        );
        assert!(game_state.len() > 500);
        let parts = vec![game_state];
        let result = summarize_tool_results(&parts, 500);
        assert!(result.contains("Delta"));
        assert!(result.contains("Resources"));
        assert!(result.len() <= 500 + 50); // some overhead from prefix
    }

    #[spec("SRV-009")]
    #[test]
    fn test_summarize_fallback_truncation() {
        let large = format!("Tool foo: {}", "x".repeat(2000));
        let parts = vec![large.clone()];
        let result = summarize_tool_results(&parts, 500);
        assert!(result.ends_with("...(summarized)"));
        assert!(result.len() < large.len());
    }

    #[spec("SRV-009")]
    #[test]
    fn test_find_matching_brace_simple() {
        assert_eq!(find_matching_brace("{\"a\":1}"), Some(6));
    }

    #[spec("SRV-009")]
    #[test]
    fn test_find_matching_brace_nested() {
        assert_eq!(find_matching_brace("{\"a\":{\"b\":1}}"), Some(12));
    }

    #[spec("SRV-009")]
    #[test]
    fn test_find_matching_brace_no_brace() {
        assert_eq!(find_matching_brace("no braces"), None);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_find_matching_brace_unbalanced() {
        assert_eq!(find_matching_brace("{\"a\":1"), None);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_condense_small_result_unchanged() {
        let result = condense_tool_result("sim_step", "{\"ok\":true}", 4000);
        assert_eq!(result, "Tool sim_step: {\"ok\":true}");
    }

    #[spec("SRV-009")]
    #[test]
    fn test_condense_extracts_delta_from_large() {
        let padding = "x".repeat(10_000);
        let large = format!(
            "{{\"Observation\":{{\"Tick\":100,\"Data\":\"{padding}\",\"Delta\":{{\"Resources\":{{\"wood\":50}}}}}}}}"
        );
        assert!(large.len() > 10_000);
        let result = condense_tool_result("sim_step", &large, 4000);
        assert!(result.contains("Delta"));
        assert!(result.contains("Resources"));
        assert!(result.len() <= 4000);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_condense_truncates_without_delta() {
        let large = "x".repeat(100_000);
        let result = condense_tool_result("foo", &large, 4000);
        assert!(result.contains("truncated"));
        assert!(result.contains("100000 total chars"));
        assert!(result.len() <= 4000);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_semantic_failure_nested() {
        // Any nested object with Success:false should be detected
        let result = r#"{"result":"{\"Data\":{\"Action\":{\"Type\":\"Create\",\"ErrorCode\":\"InternalError\",\"Message\":\"Missing required field.\",\"Success\":false}}}"}"#;
        let failure = detect_semantic_failure(result);
        assert!(failure.is_some());
        assert!(failure.unwrap().contains("Missing required field"));
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_semantic_failure_success_true() {
        let result = r#"{"result":"{\"Data\":{\"Action\":{\"Success\":true,\"Message\":\"\"}}}"}"#;
        assert!(detect_semantic_failure(result).is_none());
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_semantic_failure_no_success_field() {
        let result = r#"{"result":"{\"Tick\":100}"}"#;
        assert!(detect_semantic_failure(result).is_none());
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_semantic_failure_top_level() {
        let result = r#"{"result":"{\"Success\":false,\"Message\":\"Invalid agent ID\"}"}"#;
        let failure = detect_semantic_failure(result);
        assert!(failure.is_some());
        assert!(failure.unwrap().contains("Invalid agent ID"));
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_semantic_failure_case_insensitive() {
        let result = r#"{"result":"{\"success\":false,\"message\":\"bad request\"}"}"#;
        let failure = detect_semantic_failure(result);
        assert!(failure.is_some());
        assert!(failure.unwrap().contains("bad request"));
    }

    #[test]
    fn strip_embedded_schemas_removes_schema_arrays() {
        // Arrays of objects with "Type" + "Description" are schema-shaped
        let raw = serde_json::json!({
            "AgentId": "player1",
            "ActionSpace": {
                "Actions": (0..30).map(|i| serde_json::json!({
                    "Type": format!("Action{i}"),
                    "Description": format!("Does thing {i}")
                })).collect::<Vec<_>>(),
                "Format": "JSON"
            }
        });
        let result = super::strip_embedded_schemas(&raw.to_string());
        assert!(
            result.contains("[30 items]"),
            "Expected schema array stripped, got: {result}"
        );
        assert!(result.contains("player1"));
        assert!(result.contains("Format"));
        assert!(result.len() < raw.to_string().len() / 2);
    }

    #[test]
    fn strip_embedded_schemas_preserves_small_results() {
        let raw = r#"{"success": true, "AgentId": "player1"}"#;
        let result = super::strip_embedded_schemas(raw);
        assert!(result.contains("player1"));
        assert!(result.contains("true"));
    }

    #[test]
    fn strip_embedded_schemas_preserves_observation_data() {
        // Observation objects with runtime data (colonists, resources) must survive
        // even when they have many fields — they are NOT schemas.
        let raw = serde_json::json!({
            "result": serde_json::json!({
                "AgentId": "default",
                "Observation": {
                    "Tick": 34907,
                    "ColonistCount": 3,
                    "Colonists": [
                        {"Name": "Blas", "Health": 1.0, "Mood": 0.54, "CurrentJob": "Skygaze"},
                        {"Name": "Fernanda", "Health": 1.0, "Mood": 0.54, "CurrentJob": "Skygaze"},
                        {"Name": "Penny", "Health": 1.0, "Mood": 0.57, "CurrentJob": "Skygaze"}
                    ],
                    "Alerts": [{"Label": "Need beds", "Severity": 2}],
                    "Resources": {"Silver": 800, "Steel": 1170, "WoodLog": 300},
                    "Season": "PermanentSummer",
                    "Temperature": 28.3,
                    "Weather": "Clear",
                    "Hour": 19,
                    "IdleColonists": 0,
                    "ValidActions": ["SetSpeed", "Draft", "SetWorkPriority"],
                    "Research": {"Available": [{"Cost": 500, "DefName": "Pemmican", "Label": "pemmican"}]}
                }
            }).to_string()
        });
        let result = super::strip_embedded_schemas(&raw.to_string());
        // All colonist names must survive — planner needs them for step commands
        assert!(result.contains("Blas"), "Lost colonist name: {result}");
        assert!(result.contains("Fernanda"), "Lost colonist name: {result}");
        assert!(result.contains("Penny"), "Lost colonist name: {result}");
        // Resources must survive
        assert!(result.contains("1170"), "Lost resource data: {result}");
        // Alerts must survive
        assert!(result.contains("Need beds"), "Lost alert: {result}");
    }

    #[spec("SRV-009")]
    #[test]
    fn ungranted_tool_call_is_denied() {
        use std::collections::HashSet;
        let granted: HashSet<String> = ["git_diff", "gh_pr_review"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(tool_call_permitted("git_diff", Some(&granted)));
        assert!(tool_call_permitted("gh_pr_review", Some(&granted)));
        assert!(!tool_call_permitted("github_pr_create", Some(&granted)));
    }

    #[spec("SRV-009")]
    #[test]
    fn no_grant_set_permits_everything() {
        assert!(tool_call_permitted("anything", None));
    }
}
