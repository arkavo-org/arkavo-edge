use super::learning_bus::LearningBus;
use super::mcp_bridge::McpBridgeTool;
use super::rlm_bridge::{RlmBridge, estimate_tokens, model_context_size};
use super::tool_memory::ToolMemory;
use arkavo_hrm::{
    Conductor, TaskStore, burst::BurstResult, schemas::TaskBudget, store::InMemoryTaskStore,
};
use arkavo_mcp_tools::context_tools::{SharedRlmOps, create_context_tools};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::types::TaskProgress;
use arkavo_tasks::task_executor::TaskExecutor;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Extract a reward signal from an MCP tool result.
///
/// The result is double-wrapped JSON: `{"result": "{\"Reward\": -0.294, ...}"}`.
/// Returns `None` if the result doesn't contain a reward field.
pub(super) fn extract_reward_from_result(result_json: &str) -> Option<f64> {
    let outer: serde_json::Value = serde_json::from_str(result_json).ok()?;
    let inner_str = outer.get("result").and_then(|v| v.as_str())?;
    let inner: serde_json::Value = serde_json::from_str(inner_str).ok()?;
    inner
        .get("Reward")
        .or_else(|| inner.get("reward"))
        .and_then(|v| v.as_f64())
}

/// Execute a task using the HRM Conductor with 1:1 task-to-subtask mapping
#[allow(deprecated)] // route_with_tools bypasses architect mode, which is needed for agent tasks
pub async fn execute_with_conductor(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: String,
    task_id: Option<uuid::Uuid>,
    task_executor: Option<&Arc<TaskExecutor>>,
) -> std::result::Result<String, String> {
    execute_with_conductor_and_learning(
        conductor,
        router,
        mcp_registry,
        task_content,
        task_id,
        task_executor,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
        #[cfg(feature = "iroh")]
        None,
    )
    .await
}

/// Execute a task via HRM Conductor with optional learning and system prompt.
///
/// When `system_prompt` is provided, it is sent as a System message so the LLM
/// treats AGENTS.md instructions (tool examples, planning workflow) as authoritative.
#[allow(deprecated)] // route_with_tools bypasses architect mode, which is needed for agent tasks
#[allow(clippy::too_many_arguments)]
#[allow(clippy::implicit_hasher)] // HashSet<String> is the right type here; callers use std hasher
pub async fn execute_with_conductor_and_learning(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: String,
    task_id: Option<uuid::Uuid>,
    task_executor: Option<&Arc<TaskExecutor>>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<tokio::sync::RwLock<ToolMemory>>>,
    system_prompt: Option<&str>,
    mesh_state: Option<&Arc<arkavo_mcp_mesh::MeshToolsState>>,
    model_hint: Option<&arkavo_router::ModelChoice>,
    images: Option<Vec<String>>,
    compute_budget: Option<&arkavo_budget::SharedComputeBudget>,
    existing_messages: Option<Vec<arkavo_llm::Message>>,
    skip_complexity: bool,
    cached_registry: Option<Arc<arkavo_mcp_tools::ToolRegistry>>,
    granted_tools: Option<&std::collections::HashSet<String>>,
    #[cfg(feature = "iroh")] iroh_node: Option<&Arc<arkavo_tdf_iroh::IrohNode>>,
) -> std::result::Result<String, String> {
    use arkavo_mcp_tools::ToolRegistry;

    // Helper to update UI progress (no-op if task tracking not available)
    let update_ui_progress = |msg: &str, pct: u8| {
        if let (Some(id), Some(executor)) = (task_id, task_executor) {
            let progress = TaskProgress {
                message: Some(msg.to_string()),
                percentage: Some(pct),
                eta_seconds: None,
            };
            let executor = executor.clone();
            tokio::spawn(async move {
                let _ = executor.update_task_progress(&id, progress).await;
            });
        }
    };

    update_ui_progress("Creating task structure", 10);

    // 1. Create HRM task with default budget
    let budget = TaskBudget::default();
    let hrm_task = conductor
        .create_task(task_content.clone(), budget)
        .await
        .map_err(|e| format!("Failed to create HRM task: {e}"))?;

    info!("Created HRM task {}", hrm_task.id);

    // Helper to update both HRM intra-progress and UI progress
    let hrm_task_id = hrm_task.id;
    let update_progress = |msg: &str, pct: u8| {
        let cond = conductor.clone();
        tokio::spawn(async move {
            let _ = cond
                .update_intra_progress(hrm_task_id, pct as f64 / 100.0)
                .await;
        });
        update_ui_progress(msg, pct);
    };

    // 2a-pre. Check for autoresearch mode (parameter sweep instead of normal execution)
    if super::conductor_autoresearch::is_autoresearch_task(&task_content) {
        return super::conductor_autoresearch::execute_autoresearch_sweep(
            conductor,
            router,
            &task_content,
            learning_bus,
            compute_budget,
            model_hint,
        )
        .await;
    }

    // 2a-pre. Check for evofabric mode (AST-level code evolution)
    if super::conductor_evofabric::is_evofabric_task(&task_content) {
        return super::conductor_evofabric::execute_evofabric(
            conductor,
            router,
            &task_content,
            learning_bus,
            compute_budget,
            model_hint,
        )
        .await;
    }

    // SEQ-003: the egress guard is scoped to this task. It tracks what the
    // session ingests and refuses calls that would send it somewhere policy
    // does not allow. The workspace is the process's own directory: a write
    // inside it stays under the agent's root, anything else is a release.
    #[cfg(feature = "taint")]
    let egress_guard = {
        let session_id = hrm_task.id.to_string();
        let agent_id = learning_bus
            .map(|bus| bus.agent_id().to_string())
            .unwrap_or_else(|| session_id.clone());
        let mut destinations = arkavo_protocol::egress_destination::DestinationPolicy::new();
        if let Ok(cwd) = std::env::current_dir() {
            destinations = destinations.workspace_root(cwd);
        }
        let guard = super::egress_guard::EgressGuard::new(session_id, agent_id)
            .with_destination_policy(destinations);
        // The task text is ingested data: a secret handed to the agent in its
        // prompt must be labelled before the first tool call, not after one.
        guard.observe_input("task", &task_content);
        std::sync::Arc::new(guard)
    };

    // 2a. Check if task is complex enough to decompose into multiple subtasks.
    // Only decompose when the agent has MCP tools (external servers) — agents
    // with only mesh/built-in tools are advisors where decomposition loses context.
    let has_mcp_tools = mcp_registry
        .list_all_tools()
        .await
        .map(|t| !t.is_empty())
        .unwrap_or(false);
    let is_complex = if skip_complexity || !has_mcp_tools {
        false
    } else {
        assess_complexity_with_model(router, &task_content).await
    };
    if is_complex && has_mcp_tools {
        match super::conductor_planner::execute_with_plan(
            conductor,
            router,
            mcp_registry,
            &task_content,
            hrm_task.id,
            model_hint,
            learning_bus,
            tool_memory,
            system_prompt,
            mesh_state,
            #[cfg(feature = "taint")]
            egress_guard.clone(),
        )
        .await
        {
            Ok(result) => return Ok(result),
            Err(e) => warn!("Task decomposition failed, falling back to 1:1: {e}"),
        }
    }

    // 2b. Add single subtask (1:1 mapping — simple tasks or decomposition fallback)
    let subtask = conductor
        .add_subtask(hrm_task.id, task_content.clone(), vec![])
        .await
        .map_err(|e| format!("Failed to add subtask: {e}"))?;

    info!("Added subtask {} for task {}", subtask.id, hrm_task.id);

    // 3. Create burst contract
    let contract = conductor
        .create_contract(&subtask)
        .await
        .map_err(|e| format!("Failed to create contract: {e}"))?;

    info!("Created contract {} for subtask", contract.id);

    update_progress("Setting up tools", 25);

    // 4. Build ToolRegistry — use cached if available (same tools every cycle)
    let mut registry_arc = if let Some(cached) = cached_registry {
        debug!(
            "Using cached tool registry ({} tools)",
            cached.list_tools().len()
        );
        cached
    } else {
        let mut tool_registry = ToolRegistry::empty();

        let mcp_tools = mcp_registry
            .list_all_tools()
            .await
            .map_err(|e| format!("Failed to list MCP tools: {e}"))?;

        for tool in &mcp_tools {
            debug!(
                "Tool schema: {} - {} (params: {})",
                tool.name,
                tool.description,
                serde_json::to_string(&tool.input_schema).unwrap_or_default()
            );
        }
        let has_mcp_tools_local = !mcp_tools.is_empty();
        for tool in mcp_tools {
            let tool_name = tool.name.clone();
            let bridge = McpBridgeTool::new(mcp_registry.clone(), tool);
            tool_registry.register(&tool_name, Box::new(bridge));
        }

        if !has_mcp_tools_local && mesh_state.is_none() {
            tool_registry.register(
                "filesystem_tools",
                Box::new(arkavo_mcp_tools::filesystem::FileSystemKit::new()),
            );
            tool_registry.register(
                "git_status",
                Box::new(arkavo_mcp_tools::git::GitStatusKit::new()),
            );
            tool_registry.register(
                "git_diff",
                Box::new(arkavo_mcp_tools::git::GitDiffKit::new()),
            );
            tool_registry.register("git_log", Box::new(arkavo_mcp_tools::git::GitLogKit::new()));
            tool_registry.register(
                "test_run",
                Box::new(arkavo_mcp_tools::test_runner::TestRunnerTool::new()),
            );
            tool_registry.register(
                "shell_exec",
                Box::new(arkavo_mcp_tools::shell_exec::ShellExecTool::new()),
            );
            tool_registry.register(
                "code_review",
                Box::new(arkavo_mcp_tools::code_review::CodeReviewTool::new()),
            );
        }

        if let Some(state) = mesh_state {
            arkavo_mcp_mesh::register_tools(&mut tool_registry, state.clone());
            info!("Registered 4 mesh delegation tools");
        }

        #[cfg(feature = "iroh")]
        if let Some(node) = iroh_node {
            arkavo_mcp_tools::iroh_data::register_iroh_tools(&mut tool_registry, node.clone());
            info!("Registered 2 Iroh P2P data tools");
        }

        info!(
            "Task has {} tools available: {:?}",
            tool_registry.list_tools().len(),
            tool_registry
                .list_tools()
                .iter()
                .map(|t| &t.name)
                .collect::<Vec<_>>()
        );

        Arc::new(tool_registry)
    };

    // 4.5 Check if RLM mode should activate (large context handling)
    let input_tokens = estimate_tokens(&task_content);
    let context_size = model_context_size(
        model_hint.map(|h| h.name()),
        router.is_anthropic_available(),
    );
    let rlm_bridge = RlmBridge::with_default_manager();

    let (rlm_system_prompt, rlm_manifest_id) =
        if rlm_bridge.should_activate(input_tokens, context_size) {
            update_progress("Decomposing large context for RLM mode", 35);
            info!(
                "RLM mode activated: {} tokens > {}% of {} context",
                input_tokens, 70, context_size
            );

            match rlm_bridge.manager().decompose(&task_content).await {
                Ok(result) => {
                    info!(
                        "Context decomposed: {} chunks, {} tokens, manifest={}",
                        result.chunk_count, result.total_tokens, result.manifest_id
                    );

                    // Generate system prompt before moving bridge into Arc
                    let system_prompt = rlm_bridge.generate_system_prompt(&result);

                    // Add RLM tools to registry (rare path — clone if needed)
                    let rlm_ops: SharedRlmOps = Arc::new(rlm_bridge);
                    let context_tools = create_context_tools(rlm_ops.clone());
                    if let Some(reg) = Arc::get_mut(&mut registry_arc) {
                        for tool in context_tools {
                            let schema = tool.schema();
                            reg.register(&schema.name.clone(), tool);
                        }
                    }
                    info!("Added RLM context tools to registry");

                    (Some(system_prompt), Some(result.manifest_id))
                }
                Err(e) => {
                    warn!("RLM decomposition failed, falling back to normal mode: {e}");
                    (None, None)
                }
            }
        } else {
            debug!(
                "RLM mode not needed: {} tokens within {} context limit",
                input_tokens, context_size
            );
            (None, None)
        };

    if let Some(manifest_id) = &rlm_manifest_id {
        debug!("RLM manifest {} ready for context queries", manifest_id);
    }

    update_progress("Generating LLM response", 40);

    // 5. Execute via Router (using route_with_tools to bypass architect mode)

    // Inject learned guidance: behavior lessons + few-shot tool examples
    let augmented_content = if let Some(bus) = learning_bus {
        let tool_names: Vec<String> = registry_arc
            .list_tools()
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let few_shot_examples = bus
            .get_few_shot_examples(&tool_names, arkavo_router::learning::ToolCallFormat::Fence)
            .await;
        let behavior_guidance = bus.get_behavior_guidance(None).await;
        let behavior_lesson_count = bus.behavior_lesson_count().await;
        let domain = bus.swarm_id();
        let case_context = bus
            .get_case_context(&task_content, None, Some(domain))
            .await;

        let mut prefix = String::new();
        if !behavior_guidance.is_empty() {
            info!(
                "Injecting {} chars of behavior guidance ({} lessons)",
                behavior_guidance.len(),
                behavior_lesson_count
            );
            prefix.push_str(&behavior_guidance);
            prefix.push('\n');
        }
        if !few_shot_examples.is_empty() {
            info!(
                "Injecting {} chars of few-shot examples for {} tools",
                few_shot_examples.len(),
                tool_names.len()
            );
            prefix.push_str(&few_shot_examples);
            prefix.push('\n');
        }
        if !case_context.is_empty() {
            info!(
                "Injecting {} chars of case-based context",
                case_context.len()
            );
            prefix.push_str(&case_context);
            prefix.push('\n');
        }

        if prefix.is_empty() {
            info!(
                behavior_empty = behavior_guidance.is_empty(),
                few_shot_empty = few_shot_examples.is_empty(),
                case_empty = case_context.is_empty(),
                behavior_lesson_count,
                "No guidance prefix — all sources empty"
            );
            task_content.clone()
        } else {
            format!("{prefix}\n{task_content}")
        }
    } else {
        info!("No learning bus available for guidance injection");
        task_content.clone()
    };

    // Build messages: single System (merged) → User (task)
    // Qwen3.5 and other models require exactly one system message at the start.
    // When existing_messages is provided, use them but inject learning guidance
    // into the last user message (which is the cycle prompt).
    let messages = if let Some(mut existing) = existing_messages {
        // Find the last user message and augment it with learning guidance
        if augmented_content != task_content
            && let Some(last_user) = existing
                .iter_mut()
                .rev()
                .find(|m| m.role == arkavo_llm::Role::User)
        {
            last_user.content = augmented_content;
        }
        existing
    } else {
        let mut messages = Vec::new();
        let merged_system = match (system_prompt, &rlm_system_prompt) {
            (Some(sys), Some(rlm)) => Some(format!("{sys}\n\n{rlm}")),
            (Some(sys), None) => Some(sys.to_string()),
            (None, Some(rlm)) => Some(rlm.clone()),
            (None, None) => None,
        };
        if let Some(sys) = merged_system {
            messages.push(arkavo_llm::Message::system(sys));
        }
        if let Some(imgs) = images {
            messages.push(arkavo_llm::Message::user_with_images(
                augmented_content,
                imgs,
            ));
        } else {
            messages.push(arkavo_llm::Message::user(augmented_content));
        }
        messages
    };

    // Transition task from Pending → Running so the dashboard shows "working"
    let _ = conductor.start_task(hrm_task.id).await;

    // 6. Agentic tool loop: LLM calls tools → results fed back → LLM continues
    update_progress("Generating LLM response", 50);

    // Prepend agent purpose to task_content so the classifier sees domain
    // keywords (e.g. "code quality" → CodeReview) instead of generic cycle prompts.
    let classification_content = if let Some(purpose) = system_prompt {
        let hint = purpose.lines().next().unwrap_or(purpose);
        format!("[Context: {hint}] {task_content}")
    } else {
        task_content.clone()
    };

    // Use parallel three-track loop for all agents with tools.
    let has_any_tools = !registry_arc.list_tools().is_empty();
    let loop_result = if has_any_tools {
        super::conductor_parallel::run_tool_loop_parallel(
            router,
            &registry_arc,
            mcp_registry,
            &classification_content,
            messages,
            model_hint,
            learning_bus,
            tool_memory,
            compute_budget,
            granted_tools,
            #[cfg(feature = "taint")]
            Some(egress_guard.clone()),
        )
        .await?
    } else {
        super::conductor_tool_loop::run_tool_loop(
            router,
            &registry_arc,
            mcp_registry,
            &classification_content,
            messages,
            model_hint,
            learning_bus,
            tool_memory,
            compute_budget,
            granted_tools,
            #[cfg(feature = "taint")]
            Some(&egress_guard),
        )
        .await?
    };

    // Emit MCP-T behavior.trace for the completed task. Subject ID matches
    // the human-readable agent name used by BetaPrior/AntiPatternStore so the
    // emitted trace joins with the trust score for the same subject. Skipped
    // silently if no trust service is installed (e.g. tests, headless tools).
    if let (Some(trust_service), Some(bus)) = (arkavo_trust::current(), learning_bus) {
        super::trust_emit::emit_behavior_trace(
            &trust_service,
            bus.agent_id(),
            &hrm_task.id.to_string(),
            &loop_result.tool_observations,
            loop_result.total_latency_ms,
        );
    }

    let final_result = loop_result.final_text;
    let decision_model_name = loop_result.decision_model_name;
    let total_latency_ms = loop_result.total_latency_ms;
    let context_utilization_pct = loop_result.context_utilization_pct;

    if let Some(ref timing) = loop_result.inference_timing {
        info!(
            prompt_eval_ms = format!("{:.1}", timing.prompt_eval_ms).as_str(),
            generation_ms = format!("{:.1}", timing.generation_ms).as_str(),
            n_prompt_eval = timing.n_prompt_eval,
            n_eval = timing.n_eval,
            "LLM inference timing"
        );
        let total_inference_ms = (timing.prompt_eval_ms + timing.generation_ms) as u64;
        arkavo_observability::subsystem_timing::global_timing()
            .inference
            .record(total_inference_ms);
    }
    info!(
        context_tokens = loop_result.context_tokens,
        context_utilization_pct = format!("{context_utilization_pct:.1}").as_str(),
        "Context window utilization"
    );
    // Store current context utilization on the per-agent ToolMemory for telemetry
    if let Some(tm) = tool_memory {
        tm.write()
            .await
            .set_context_utilization(context_utilization_pct);
    }

    // 7. Record result in Conductor
    use arkavo_router::selector_quality::compute_response_quality;
    let quality_category = if loop_result.tool_call_count > 0 {
        "tool_use"
    } else {
        "general"
    };
    // Post-tool-loop scoring of the whole task outcome. Tools-required
    // detection happens upstream in quality_gate per-inference; here we
    // pass `false` so this aggregate score isn't double-penalized when an
    // earlier round legitimately had no tool calls (e.g., final summary
    // response).
    let response_quality = compute_response_quality(
        &final_result,
        0,
        quality_category,
        loop_result.tool_call_count,
        false,
    );

    let burst_result =
        BurstResult::success(contract.id, serde_json::json!({ "content": final_result }));
    conductor
        .record_result(hrm_task.id, subtask.id, burst_result)
        .await
        .map_err(|e| format!("Failed to record result: {e}"))?;

    // Store quality score on the subtask result for UI reporting
    if let Ok(mut task) = conductor.get_task(hrm_task.id).await {
        if let Some(st) = task.subtasks.iter_mut().find(|s| s.id == subtask.id)
            && let Some(ref mut result) = st.result
        {
            result.quality_score = Some(response_quality);
        }
        let _ = conductor.store().save(&task).await;
    }

    // 8. Retrospective credit assignment via FinalTaskReport
    //
    // Compute per-step quality from the overall response and feed it into
    // Thompson Sampling so models get credit proportional to actual outcome
    // quality (not just binary success/failure).
    if let Some(model_name) = &decision_model_name {
        use arkavo_router::learning::{AgentContribution, FinalTaskReport};
        let contributions = vec![AgentContribution {
            agent_id: model_name.clone(),
            position: 0,
            immediate_reward: response_quality,
        }];

        let report =
            FinalTaskReport::success(hrm_task.id, contributions).with_reward(response_quality);

        router.model_learning().retrospective_update(&report).await;

        // Feed latency into Thompson Sampling via BurstFeedback
        let latency_feedback = arkavo_router::BurstFeedback::success(
            hrm_task.id,
            "general".to_string(),
            total_latency_ms,
        )
        .with_quality(response_quality);
        router
            .model_learning()
            .immediate_update(model_name, &latency_feedback)
            .await;

        // Wire quality into policy cache for trend tracking + guidance
        if let Some(bus) = learning_bus {
            bus.record_quality(model_name, "general", response_quality)
                .await;

            // Feed low-quality results into AutoLearner pain aggregation
            if response_quality < 0.5 {
                let desc = format!(
                    "Task quality {:.0}% for model {model_name}",
                    response_quality * 100.0
                );
                bus.report_autolearn_pain(1.0 - response_quality, model_name, &desc);
            }
        }

        // Bypass episode buffer for critically low quality — learn immediately
        if response_quality < 0.1
            && let Some(bus) = learning_bus
        {
            use arkavo_router::learning::{Lesson, LessonPattern};
            let warning = "Action produced 0% quality. The last approach failed completely. \
                 Try a fundamentally different strategy next time."
                .to_string();
            let lesson = Lesson::new(
                model_name.clone(),
                "default".to_string(),
                "general".to_string(),
                LessonPattern::new(
                    format!("quality dropped to {:.0}%", response_quality * 100.0),
                    warning,
                    "agent tries different approach".to_string(),
                ),
                0.9,
                1,
            );
            bus.add_lesson_to_cache(lesson).await;
        }
    }

    update_progress("Finalizing", 95);

    Ok(final_result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-009")]
    #[test]
    fn extract_reward_positive() {
        let json = r#"{"result":"{\"Reward\":0.5,\"State\":{}}"}"#;
        assert_eq!(extract_reward_from_result(json), Some(0.5));
    }

    #[spec("SRV-009")]
    #[test]
    fn extract_reward_negative() {
        let json = r#"{"result":"{\"Reward\":-0.294,\"food_critical\":-0.2}"}"#;
        let reward = extract_reward_from_result(json).unwrap();
        assert!((reward - (-0.294)).abs() < 1e-6);
    }

    #[spec("SRV-009")]
    #[test]
    fn extract_reward_lowercase_key() {
        let json = r#"{"result":"{\"reward\":1.0}"}"#;
        assert_eq!(extract_reward_from_result(json), Some(1.0));
    }

    #[spec("SRV-009")]
    #[test]
    fn extract_reward_missing() {
        let json = r#"{"result":"{\"State\":{\"colonists\":3}}"}"#;
        assert_eq!(extract_reward_from_result(json), None);
    }

    #[spec("SRV-009")]
    #[test]
    fn extract_reward_not_double_wrapped() {
        let json = r#"{"Reward":0.5}"#;
        assert_eq!(extract_reward_from_result(json), None);
    }

    #[spec("SRV-009")]
    #[test]
    fn extract_reward_invalid_json() {
        assert_eq!(extract_reward_from_result("not json"), None);
    }

    #[spec("SRV-009")]
    #[test]
    fn reward_to_quality_mapping() {
        // Reward -0.294 → quality midpoint(-0.294, 1.0) = 0.353
        let quality = f64::midpoint(-0.294, 1.0).clamp(0.0, 1.0);
        assert!((quality - 0.353).abs() < 0.001);

        // Reward 0.0 → quality 0.5
        let quality_zero = f64::midpoint(0.0, 1.0).clamp(0.0, 1.0);
        assert!((quality_zero - 0.5).abs() < 1e-6);

        // Reward 1.0 → quality 1.0
        let quality_max = f64::midpoint(1.0, 1.0).clamp(0.0, 1.0);
        assert!((quality_max - 1.0).abs() < 1e-6);

        // Reward -1.0 → quality 0.0
        let quality_min = f64::midpoint(-1.0, 1.0).clamp(0.0, 1.0);
        assert!(quality_min.abs() < 1e-6);
    }
}

/// Use the smallest loaded model to assess whether a task needs decomposition.
/// Returns true only when the model explicitly says MULTI — defaults to SINGLE
/// on ambiguity, timeout, or error (false negatives are cheap, false positives
/// cause 80+ second decomposition overhead).
async fn assess_complexity_with_model(
    router: &Arc<arkavo_router::Router>,
    task_content: &str,
) -> bool {
    // Truncate to avoid wasting tokens on long cycle prompts
    let snippet = if task_content.len() > 400 {
        &task_content[..400]
    } else {
        task_content
    };

    let prompt = format!(
        "Does this require breaking into SEPARATE INDEPENDENT subtasks that \
         could be planned in isolation? A single task with multiple steps \
         (like 'register then observe then act') is SINGLE. Only say MULTI \
         if there are truly independent goals.\n\
         Reply SINGLE or MULTI.\n\n\
         Task: {snippet}"
    );

    let messages = vec![arkavo_llm::Message::user(&prompt)];

    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        router.route_chat(messages, None, None),
    )
    .await
    {
        Ok(Ok(response)) => {
            let answer = response.content.to_lowercase();
            let is_multi = answer.contains("multi") && !answer.contains("single");
            info!(
                answer = %response.content.trim(),
                is_multi,
                task_len = task_content.len(),
                "LLM complexity assessment"
            );
            is_multi
        }
        Ok(Err(e)) => {
            info!("Complexity model error, defaulting to SINGLE: {e}");
            false
        }
        Err(_) => {
            info!("Complexity model timeout (10s), defaulting to SINGLE");
            false
        }
    }
}
