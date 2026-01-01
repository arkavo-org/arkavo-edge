use super::learning_bus::{LearningBus, LearningEvent};
use super::mcp_bridge::McpBridgeTool;
use crate::mcp_registry::McpRegistry;
use crate::task_executor::TaskExecutor;
use crate::types::TaskProgress;
use arkavo_hrm::{Conductor, burst::BurstResult, schemas::TaskBudget, store::InMemoryTaskStore};
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
    execute_with_conductor_full(
        conductor,
        router,
        mcp_registry,
        task_content,
        task_id,
        task_executor,
        None,
        None,
    )
    .await
}

/// Execute a task using the HRM Conductor with a preferred model
#[allow(deprecated)]
pub(super) async fn execute_with_conductor_and_model(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: String,
    preferred_model: Option<arkavo_router::ModelChoice>,
) -> std::result::Result<String, String> {
    execute_with_conductor_full(
        conductor,
        router,
        mcp_registry,
        task_content,
        None,
        None,
        None,
        preferred_model,
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
    execute_with_conductor_full(
        conductor,
        router,
        mcp_registry,
        task_content,
        task_id,
        task_executor,
        learning_bus,
        None,
    )
    .await
}

/// Full conductor execution with all options
#[allow(deprecated)]
pub(super) async fn execute_with_conductor_full(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: String,
    task_id: Option<uuid::Uuid>,
    task_executor: Option<&Arc<TaskExecutor>>,
    learning_bus: Option<&Arc<LearningBus>>,
    preferred_model: Option<arkavo_router::ModelChoice>,
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

    let messages = vec![arkavo_llm::Message {
        role: arkavo_llm::Role::User,
        content: augmented_content,
        images: None,
    }];

    let response = router
        .route_with_tools_and_model(&task_content, messages, Some(&registry_arc), preferred_model)
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

            // Call the tool via MCP registry - convert error to String early to avoid Send issues
            let tool_result = mcp_registry
                .call_tool(&tool_call.tool_name, args.clone(), "hrm")
                .await
                .map_err(|e| e.to_string());

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
                    // Treat "already" errors as idempotent success (e.g., "already registered")
                    let err_lower = err_str.to_lowercase();
                    if err_lower.contains("already ") {
                        info!(
                            "Tool {} - idempotent success: {}",
                            tool_call.tool_name, err_str
                        );
                        tool_results.push(format!(
                            "## Tool: {}\nSUCCESS: Already completed. Do NOT call this tool again. Proceed with next action.",
                            tool_call.tool_name
                        ));
                        continue;
                    }

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
