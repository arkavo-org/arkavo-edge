use super::learning_bus::{LearningBus, LearningEvent};
use super::mcp_bridge::McpBridgeTool;
use super::rlm_bridge::{RlmBridge, estimate_tokens, model_context_size};
use super::tool_memory::ToolMemory;
use arkavo_hrm::{Conductor, burst::BurstResult, schemas::TaskBudget, store::InMemoryTaskStore};
use arkavo_mcp_tools::context_tools::{SharedRlmOps, create_context_tools};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::types::TaskProgress;
use arkavo_router::BurstFeedback;
use arkavo_tasks::task_executor::TaskExecutor;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Extract a reward signal from an MCP tool result.
///
/// The result is double-wrapped JSON: `{"result": "{\"Reward\": -0.294, ...}"}`.
/// Returns `None` if the result doesn't contain a reward field.
fn extract_reward_from_result(result_json: &str) -> Option<f64> {
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
    )
    .await
}

/// Execute a task via HRM Conductor with optional learning and system prompt.
///
/// When `system_prompt` is provided, it is sent as a System message so the LLM
/// treats AGENTS.md instructions (tool examples, planning workflow) as authoritative.
#[allow(deprecated)] // route_with_tools bypasses architect mode, which is needed for agent tasks
#[allow(clippy::too_many_arguments)]
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
) -> std::result::Result<String, String> {
    use arkavo_mcp_tools::ToolRegistry;

    // Helper to update progress (no-op if task tracking not available)
    let update_progress = |msg: &str, pct: u8| {
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

    update_progress("Creating task structure", 10);

    // 1. Create HRM task with default budget
    let budget = TaskBudget::default();
    let hrm_task = conductor
        .create_task(task_content.clone(), budget)
        .await
        .map_err(|e| format!("Failed to create HRM task: {e}"))?;

    info!("Created HRM task {}", hrm_task.id);

    // 2. Add single subtask (1:1 mapping)
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

    // 4. Project MCP tools to ToolRegistry for Router
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
    for tool in mcp_tools {
        // Create bridge tool that delegates to MCP registry
        let tool_name = tool.name.clone();
        let bridge = McpBridgeTool::new(mcp_registry.clone(), tool);
        tool_registry.register(&tool_name, Box::new(bridge));
    }

    // Register A2A mesh tools (list_agents, agent_query, send_task, get_task_status)
    if let Some(state) = mesh_state {
        arkavo_mcp_mesh::register_tools(&mut tool_registry, state.clone());
        info!("Registered 4 mesh delegation tools");
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

    // 4.5 Check if RLM mode should activate (large context handling)
    let input_tokens = estimate_tokens(&task_content);
    let context_size = model_context_size(None, router.is_anthropic_available());
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

                    // Add RLM tools to registry
                    let rlm_ops: SharedRlmOps = Arc::new(rlm_bridge);
                    let context_tools = create_context_tools(rlm_ops.clone());
                    for tool in context_tools {
                        let schema = tool.schema();
                        tool_registry.register(&schema.name.clone(), tool);
                    }
                    info!("Added 3 RLM context tools to registry");

                    // Generate system prompt with manifest reference
                    // Recreate bridge since we moved it into Arc
                    let bridge_for_prompt = RlmBridge::with_default_manager();
                    let system_prompt = bridge_for_prompt.generate_system_prompt(&result);

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
    let registry_arc = Arc::new(tool_registry);

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

        let mut prefix = String::new();
        if !behavior_guidance.is_empty() {
            info!(
                "Injecting {} chars of behavior guidance",
                behavior_guidance.len()
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

        if prefix.is_empty() {
            task_content.clone()
        } else {
            format!("{prefix}\n{task_content}")
        }
    } else {
        task_content.clone()
    };

    // Build messages: System (AGENTS.md purpose) → System (RLM, if active) → User (task)
    let mut messages = Vec::new();
    if let Some(sys) = system_prompt {
        messages.push(arkavo_llm::Message {
            role: arkavo_llm::Role::System,
            content: sys.to_string(),
            images: None,
        });
    }
    if let Some(ref rlm_prompt) = rlm_system_prompt {
        messages.push(arkavo_llm::Message {
            role: arkavo_llm::Role::System,
            content: rlm_prompt.clone(),
            images: None,
        });
    }
    messages.push(arkavo_llm::Message {
        role: arkavo_llm::Role::User,
        content: augmented_content,
        images,
    });

    let response = router
        .route_with_tools_hinted(&task_content, messages, Some(&registry_arc), model_hint)
        .await
        .map_err(|e| format!("Router failed: {e}"))?;

    info!("LLM response received, {} chars", response.content.len());

    update_progress("Processing response", 60);

    // Debug: show full LLM response for tool call debugging
    if std::env::var("ARKAVO_DEBUG").is_ok() {
        eprintln!("[LLM Response] {} chars:", response.content.len());
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
    debug!(
        "LLM returned {} tool calls: {:?}",
        response.tool_calls.len(),
        response
            .tool_calls
            .iter()
            .map(|tc| format!("{}({})", tc.tool_name, tc.arguments))
            .collect::<Vec<_>>()
    );

    // 6. Handle any tool calls returned by the LLM
    let mut final_result = response.content.clone();

    if !response.tool_calls.is_empty() {
        let tool_count = response.tool_calls.len();
        update_progress(&format!("Executing {tool_count} tool calls"), 70);
        info!("Executing {tool_count} tool calls");

        let mut tool_results = Vec::new();
        let mut reward_signals: Vec<f64> = Vec::new();
        for tool_call in &response.tool_calls {
            let args = tool_call.arguments.clone();
            debug!(
                "Tool call: {} with args: {}",
                tool_call.tool_name,
                serde_json::to_string(&args).unwrap_or_default()
            );

            let start_time = std::time::Instant::now();

            // Try ToolRegistry first (includes RLM context tools), then fall back to MCP registry
            let tool_result = if let Some(tool) = registry_arc.get(&tool_call.tool_name) {
                tool.execute(args.clone()).await.map_err(|e| e.to_string())
            } else {
                // Fall back to MCP registry for external tools
                mcp_registry
                    .call_tool(&tool_call.tool_name, args.clone(), "hrm")
                    .await
                    .map_err(|e| e.to_string())
            };

            match tool_result {
                Ok(result) => {
                    let latency_ms = start_time.elapsed().as_millis() as u64;
                    let result_str = serde_json::to_string(&result).unwrap_or_default();

                    // Extract reward signal from game/sim results.
                    // Negative reward means the action hurt (e.g. colonists starving)
                    // even though the MCP call itself succeeded.
                    let reward = extract_reward_from_result(&result_str);
                    let tool_success = reward.is_none_or(|r| r >= 0.0);

                    if let Some(r) = reward {
                        reward_signals.push(r);
                        if r < 0.0 {
                            info!(
                                "Tool {} returned negative reward {:.3} — marking as failure for learning",
                                tool_call.tool_name, r
                            );
                        }
                    } else {
                        info!("Tool {} succeeded", tool_call.tool_name);
                    }
                    debug!("Tool {} result: {}", tool_call.tool_name, result_str);

                    // Record in short-term tool memory
                    if let Some(mem) = tool_memory {
                        mem.write()
                            .await
                            .add(tool_call.tool_name.clone(), &args, &result_str);
                    }

                    // Emit learning event — success reflects game reward, not just MCP status
                    if let Some(bus) = learning_bus {
                        let event = LearningEvent::ToolCall {
                            tool_name: tool_call.tool_name.clone(),
                            args: args.clone(),
                            result: result_str.clone(),
                            success: tool_success,
                            latency_ms,
                        };
                        let _ = bus.sender().send(event).await;
                    }

                    tool_results.push(format!(
                        "## Tool: {}\n{}",
                        tool_call.tool_name,
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    ));
                }
                Err(err_str) => {
                    let latency_ms = start_time.elapsed().as_millis() as u64;
                    warn!("Tool {} failed: {}", tool_call.tool_name, err_str);

                    // Record failure in short-term tool memory
                    if let Some(mem) = tool_memory {
                        mem.write().await.add(
                            tool_call.tool_name.clone(),
                            &args,
                            &format!("Error: {err_str}"),
                        );
                    }

                    // Emit learning event for failed tool call
                    if let Some(bus) = learning_bus {
                        let event = LearningEvent::ToolCall {
                            tool_name: tool_call.tool_name.clone(),
                            args: args.clone(),
                            result: format!("Error: {err_str}"),
                            success: false,
                            latency_ms,
                        };
                        let _ = bus.sender().send(event).await;
                    }

                    // Learn tool error correction for future prompts
                    if let Some(model_name) = router.last_routed_model() {
                        let model_family = arkavo_router::Router::detect_model_family(&model_name);
                        router.advisor().observe_tool_error(
                            &model_family,
                            &tool_call.tool_name,
                            &err_str,
                            &args,
                        );
                    }

                    tool_results.push(format!(
                        "## Tool: {} (Error)\n{}",
                        tool_call.tool_name, err_str
                    ));
                }
            }
        }

        if !tool_results.is_empty() {
            final_result.push_str("\n\n## Tool Execution Results\n");
            final_result.push_str(&tool_results.join("\n\n"));
        }

        // Corrective Thompson Sampling feedback based on actual game reward.
        // The quality gate already recorded success + quality ~1.0 based on text format.
        // This second signal injects the ground-truth reward so Thompson Sampling
        // eventually demotes models that produce plausible-looking but harmful actions.
        if !reward_signals.is_empty()
            && let Some(model_name) = router.last_routed_model()
        {
            let avg_reward = reward_signals.iter().sum::<f64>() / reward_signals.len() as f64;
            // Map reward [-1, 1] → quality [0, 1]
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
        }
    } else {
        debug!("LLM did not request any tool calls");
    }

    // 7. Record result in Conductor
    let burst_result =
        BurstResult::success(contract.id, serde_json::json!({ "content": final_result }));
    conductor
        .record_result(hrm_task.id, subtask.id, burst_result)
        .await
        .map_err(|e| format!("Failed to record result: {e}"))?;

    update_progress("Finalizing", 95);

    info!("Task {} completed via Conductor", hrm_task.id);

    Ok(final_result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_reward_positive() {
        let json = r#"{"result":"{\"Reward\":0.5,\"State\":{}}"}"#;
        assert_eq!(extract_reward_from_result(json), Some(0.5));
    }

    #[test]
    fn extract_reward_negative() {
        let json = r#"{"result":"{\"Reward\":-0.294,\"food_critical\":-0.2}"}"#;
        let reward = extract_reward_from_result(json).unwrap();
        assert!((reward - (-0.294)).abs() < 1e-6);
    }

    #[test]
    fn extract_reward_lowercase_key() {
        let json = r#"{"result":"{\"reward\":1.0}"}"#;
        assert_eq!(extract_reward_from_result(json), Some(1.0));
    }

    #[test]
    fn extract_reward_missing() {
        let json = r#"{"result":"{\"State\":{\"colonists\":3}}"}"#;
        assert_eq!(extract_reward_from_result(json), None);
    }

    #[test]
    fn extract_reward_not_double_wrapped() {
        let json = r#"{"Reward":0.5}"#;
        assert_eq!(extract_reward_from_result(json), None);
    }

    #[test]
    fn extract_reward_invalid_json() {
        assert_eq!(extract_reward_from_result("not json"), None);
    }

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
