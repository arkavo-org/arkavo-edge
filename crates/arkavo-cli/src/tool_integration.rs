#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_llm::{Message, ProviderResponse, ToolExecutionResult, ToolExecutor};
#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_mcp_tools::{McpClient as McpClientTrait, ToolRegistry};
#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_router::Router;
#[cfg(all(unix, feature = "mcp-tools"))]
use std::sync::Arc;

#[cfg(not(all(unix, feature = "mcp-tools")))]
use arkavo_llm::Message;

#[cfg(all(unix, feature = "mcp-tools"))]
pub struct ToolIntegrationConfig {
    pub max_tool_iterations: usize,
    pub show_tool_execution: bool,
}

#[cfg(all(unix, feature = "mcp-tools"))]
impl Default for ToolIntegrationConfig {
    fn default() -> Self {
        Self {
            max_tool_iterations: 3,
            show_tool_execution: true,
        }
    }
}

#[cfg(all(unix, feature = "mcp-tools"))]
pub struct ToolIntegrationResult {
    pub final_response: String,
    pub tool_executions: Vec<ToolExecutionResult>,
    pub total_iterations: usize,
}

/// Process messages with automatic tool calling via Router
///
/// This function:
/// 1. Routes the request through arkavo-router for model selection
/// 2. Passes MCP tool definitions to the selected LLM
/// 3. Executes any tool calls returned by the LLM
/// 4. Feeds tool results back to the LLM for final response
/// 5. Repeats until no more tool calls or max iterations reached
#[cfg(all(unix, feature = "mcp-tools"))]
pub async fn process_with_tools(
    task_description: &str,
    mut messages: Vec<Message>,
    config: Option<ToolIntegrationConfig>,
    mcp_client: Option<Arc<dyn McpClientTrait>>,
) -> Result<ToolIntegrationResult, Box<dyn std::error::Error>> {
    let config = config.unwrap_or_default();

    let router = Router::new().await?;

    let tool_registry = ToolRegistry::from_mcp_or_default(mcp_client);
    let tool_executor = ToolExecutor::new();

    let mut all_tool_executions = Vec::new();
    let mut iteration = 0;

    loop {
        iteration += 1;

        if iteration > config.max_tool_iterations {
            return Err(format!(
                "Maximum tool iterations ({}) exceeded",
                config.max_tool_iterations
            )
            .into());
        }

        let response: ProviderResponse = router
            .route_with_tools(task_description, messages.clone(), Some(&tool_registry))
            .await?;

        if response.tool_calls.is_empty() {
            return Ok(ToolIntegrationResult {
                final_response: response.content,
                tool_executions: all_tool_executions,
                total_iterations: iteration,
            });
        }

        if config.show_tool_execution {
            println!("\n=== Tool Execution (Iteration {}) ===", iteration);
            println!("LLM wants to call {} tool(s)", response.tool_calls.len());
        }

        let mut tool_results = Vec::new();

        for tool_call in &response.tool_calls {
            if config.show_tool_execution {
                println!("\nExecuting tool: {}", tool_call.tool_name);
                if let Ok(args_pretty) = serde_json::to_string_pretty(&tool_call.arguments) {
                    println!("Arguments:\n{}", args_pretty);
                }
            }

            let result = tool_executor.execute(tool_call).await?;

            if config.show_tool_execution {
                println!(
                    "Result: {}",
                    if result.success {
                        "✓ Success"
                    } else {
                        "✗ Failed"
                    }
                );
                if let Ok(result_pretty) = serde_json::to_string_pretty(&result.result) {
                    println!("{}", result_pretty);
                }
            }

            tool_results.push(result.clone());
            all_tool_executions.push(result);
        }

        messages.push(Message::assistant(&response.content));

        let tool_results_message = format_tool_results(&tool_results);
        messages.push(Message::user(&tool_results_message));

        if config.show_tool_execution {
            println!("\n=== Feeding results back to LLM ===\n");
        }
    }
}

#[cfg(all(unix, feature = "mcp-tools"))]
fn format_tool_results(results: &[ToolExecutionResult]) -> String {
    let mut formatted = String::from("Tool execution results:\n\n");

    for result in results {
        formatted.push_str(&format!("Tool: {}\n", result.tool_name));
        if result.success {
            formatted.push_str(&format!(
                "Result: {}\n",
                serde_json::to_string_pretty(&result.result).unwrap_or_else(|_| "{}".to_string())
            ));
        } else {
            formatted.push_str(&format!(
                "Error: {}\n",
                result.error.as_deref().unwrap_or("Unknown error")
            ));
        }
        formatted.push('\n');
    }

    formatted
}

/// Simplified version for non-interactive use (task command, A2A, etc.)
#[cfg(all(unix, feature = "mcp-tools"))]
pub async fn complete_with_tools(
    task_description: &str,
    messages: Vec<Message>,
    mcp_client: Option<Arc<dyn McpClientTrait>>,
) -> Result<String, Box<dyn std::error::Error>> {
    let config = ToolIntegrationConfig {
        max_tool_iterations: 3,
        show_tool_execution: false,
    };

    let result = process_with_tools(task_description, messages, Some(config), mcp_client).await?;
    Ok(result.final_response)
}

/// Interactive version for chat command that shows streaming output
///
/// This version:
/// 1. Routes through arkavo-router for model selection
/// 2. Gets non-streaming response with tool calls
/// 3. Executes tools if needed
/// 4. Continues iteration until no more tool calls
/// 5. Returns final response for display
#[cfg(all(unix, feature = "mcp-tools"))]
pub async fn process_with_tools_interactive(
    task_description: &str,
    messages: Vec<Message>,
    mcp_client: Option<Arc<dyn McpClientTrait>>,
) -> Result<String, Box<dyn std::error::Error>> {
    let config = ToolIntegrationConfig {
        max_tool_iterations: 3,
        show_tool_execution: true,
    };

    let result = process_with_tools(task_description, messages, Some(config), mcp_client).await?;
    Ok(result.final_response)
}

/// Fallback for interactive mode on unsupported platforms
#[cfg(not(all(unix, feature = "mcp-tools")))]
pub async fn process_with_tools_interactive(
    _task_description: &str,
    _messages: Vec<Message>,
) -> Result<String, Box<dyn std::error::Error>> {
    Err("Tool integration requires Unix platform with mcp-tools feature".into())
}

/// Fallback for platforms without tool support
#[cfg(not(all(unix, feature = "mcp-tools")))]
pub async fn complete_with_tools(
    _task_description: &str,
    _messages: Vec<Message>,
    _mcp_client: Option<std::sync::Arc<dyn Send + Sync>>,
) -> Result<String, Box<dyn std::error::Error>> {
    Err("Tool integration requires Unix platform with mcp-tools feature".into())
}
