use super::learning_bus::{LearningBus, LearningEvent};
use super::mcp_bridge::McpBridgeTool;
use super::rlm_bridge::{RlmBridge, estimate_tokens, model_context_size};
use crate::mcp_registry::McpRegistry;
use crate::task_executor::TaskExecutor;
use crate::types::TaskProgress;
use arkavo_hrm::{Conductor, burst::BurstResult, schemas::TaskBudget, store::InMemoryTaskStore};
use arkavo_mcp_tools::context_tools::{SharedRlmOps, create_context_tools};
use std::sync::Arc;
use tracing::{debug, info, warn};

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
    )
    .await
}

/// Execute a task using the HRM Conductor with 1:1 task-to-subtask mapping
/// Optionally emits learning events for tool calls
#[allow(deprecated)] // route_with_tools bypasses architect mode, which is needed for agent tasks
pub async fn execute_with_conductor_and_learning(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: String,
    task_id: Option<uuid::Uuid>,
    task_executor: Option<&Arc<TaskExecutor>>,
    learning_bus: Option<&Arc<LearningBus>>,
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

    let tool_count = mcp_tools.len();
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

    info!(
        "Task has {} MCP tools available: {:?}",
        tool_count,
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

    // Inject few-shot examples from learned tool patterns
    let augmented_content = if let Some(bus) = learning_bus {
        let tool_names: Vec<String> = registry_arc
            .list_tools()
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let few_shot_examples = bus
            .get_few_shot_examples(&tool_names, arkavo_router::learning::ToolCallFormat::Fence)
            .await;

        if !few_shot_examples.is_empty() {
            info!(
                "Injecting {} chars of few-shot examples for {} tools",
                few_shot_examples.len(),
                tool_names.len()
            );
            format!("{few_shot_examples}\n\n{task_content}")
        } else {
            task_content.clone()
        }
    } else {
        task_content.clone()
    };

    // Build messages, prepending RLM system prompt if active
    let messages = if let Some(ref rlm_prompt) = rlm_system_prompt {
        vec![
            arkavo_llm::Message {
                role: arkavo_llm::Role::System,
                content: rlm_prompt.clone(),
                images: None,
            },
            arkavo_llm::Message {
                role: arkavo_llm::Role::User,
                content: augmented_content,
                images: None,
            },
        ]
    } else {
        vec![arkavo_llm::Message {
            role: arkavo_llm::Role::User,
            content: augmented_content,
            images: None,
        }]
    };

    let response = router
        .route_with_tools(&task_content, messages, Some(&registry_arc))
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
                    info!("Tool {} succeeded", tool_call.tool_name);
                    let result_str = serde_json::to_string(&result).unwrap_or_default();
                    debug!("Tool {} result: {}", tool_call.tool_name, result_str);

                    // Emit learning event for successful tool call
                    if let Some(bus) = learning_bus {
                        let event = LearningEvent::ToolCall {
                            tool_name: tool_call.tool_name.clone(),
                            args: args.clone(),
                            result: result_str.clone(),
                            success: true,
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
