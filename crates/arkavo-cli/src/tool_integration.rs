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
            max_tool_iterations: 10,
            show_tool_execution: false,
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
/// 1. Checks task complexity - uses architect mode for complex multi-step tasks
/// 2. Routes the request through arkavo-router for model selection
/// 3. Passes MCP tool definitions to the selected LLM
/// 4. Executes any tool calls returned by the LLM
/// 5. Feeds tool results back to the LLM for final response
/// 6. Repeats until no more tool calls or max iterations reached
#[cfg(all(unix, feature = "mcp-tools"))]
pub async fn process_with_tools(
    task_description: &str,
    mut messages: Vec<Message>,
    config: Option<ToolIntegrationConfig>,
    mcp_client: Option<Arc<dyn McpClientTrait>>,
) -> Result<ToolIntegrationResult, Box<dyn std::error::Error>> {
    let config = config.unwrap_or_default();

    let router = Arc::new(Router::new().await?);

    let mut tool_registry = ToolRegistry::from_mcp_or_default(mcp_client);

    // Register tools from each crate
    arkavo_router::tools::register_tools(&mut tool_registry, router.clone());

    // Register mesh orchestration tools for agent coordination
    let mesh_state = std::sync::Arc::new(arkavo_mesh_tools::MeshToolsState::new());
    arkavo_mesh_tools::register_tools(&mut tool_registry, mesh_state);

    // Wrap registry in Arc for shared access
    let registry_arc = Arc::new(tool_registry);

    // Create executor with the same registry that has router tools registered
    let tool_executor = ToolExecutor::with_registry(registry_arc.clone());

    let mut all_tool_executions = Vec::new();
    let mut iteration = 0;
    let mut first_call = true;

    loop {
        // First call uses route() which handles architect mode detection
        // Subsequent calls use route_with_quality_gate() directly
        let response: ProviderResponse = if first_call {
            first_call = false;
            let stream = router
                .route(task_description, messages.clone(), Some(&registry_arc))
                .await?;

            // Check if architect mode was used - if so, we're done
            if stream.metadata().used_architect_mode {
                let result = stream.complete().await?;
                if let Some(savings) = result.architect_savings {
                    println!("Architect mode: saved ${savings:.4}");
                }
                return Ok(ToolIntegrationResult {
                    final_response: result.content,
                    tool_executions: Vec::new(),
                    total_iterations: 1,
                });
            }

            // Not architect mode - convert RouteResponse to ProviderResponse
            let result = stream.complete().await?;
            ProviderResponse {
                content: result.content,
                reasoning_content: None,
                tool_calls: result.tool_calls,
                finish_reason: None,
            }
        } else {
            // Subsequent iterations - use quality gate directly
            #[allow(deprecated)]
            match router
                .route_with_quality_gate(task_description, messages.clone(), Some(&registry_arc), 3)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    // Check if judge detected missing tool usage
                    let error_msg = e.to_string();
                    if error_msg.starts_with("MISSING_TOOL_USE:") {
                        // Tool discovery doesn't count as an iteration
                        // Extract keywords from error message
                        let keywords_str =
                            error_msg.strip_prefix("MISSING_TOOL_USE:").unwrap_or("");
                        let keywords: Vec<String> = keywords_str
                            .trim_matches(|c| c == '[' || c == ']' || c == '"')
                            .split(',')
                            .map(|s| s.trim().trim_matches('"').to_string())
                            .filter(|s| !s.is_empty())
                            .collect();

                        tracing::info!(
                            "Judge detected missing tool usage, searching for: {:?}",
                            keywords
                        );

                        // Search for tools matching the judge's suggested keywords
                        let mut expanded_tools = Vec::new();
                        for keyword in &keywords {
                            let found = registry_arc
                                .search_tools(keyword, arkavo_mcp_tools::DetailLevel::FullSchema);
                            tracing::debug!("Keyword '{}' matched {} tools", keyword, found.len());
                            expanded_tools.extend(found);
                        }

                        // Log if no tools were found
                        if expanded_tools.is_empty() {
                            tracing::warn!(
                                target: "arkavo_tools::judge_keyword_miss",
                                keywords = ?keywords,
                                "Judge suggested keywords but no tools matched"
                            );
                        }

                        // Feed back the tool definitions to the LLM
                        let tool_list = expanded_tools
                            .iter()
                            .map(|t| {
                                let aliases_text = if let Some(aliases) = &t.aliases {
                                    if !aliases.is_empty() {
                                        format!(" (aliases: {})", aliases.join(", "))
                                    } else {
                                        String::new()
                                    }
                                } else {
                                    String::new()
                                };
                                format!(
                                    "- {}{}: {}",
                                    t.name,
                                    aliases_text,
                                    t.description.as_deref().unwrap_or("No description")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        let tool_response = if expanded_tools.is_empty() {
                            "No matching tools found for the requested information.".to_string()
                        } else {
                            format!(
                                "Found {} relevant tool(s):\n{}\n\nPlease use these tools to answer the question.",
                                expanded_tools.len(),
                                tool_list
                            )
                        };

                        messages.push(Message::user(&tool_response));
                        continue; // Re-route with expanded knowledge (doesn't count as iteration)
                    }

                    // Not a missing tool error, propagate it
                    return Err(e.into());
                }
            }
        };

        // Check if LLM is requesting tools via REQUEST_TOOL protocol
        let requested_keywords =
            arkavo_router::tool_request_parser::parse_tool_requests(&response.content);
        if !requested_keywords.is_empty() {
            // Tool metadata requests don't count as iterations
            tracing::info!("LLM requested tools via keywords: {:?}", requested_keywords);

            // Search for tools matching the requested keywords
            let mut expanded_tools = Vec::new();
            for keyword in &requested_keywords {
                let found =
                    registry_arc.search_tools(keyword, arkavo_mcp_tools::DetailLevel::FullSchema);
                tracing::debug!("Keyword '{}' matched {} tools", keyword, found.len());
                expanded_tools.extend(found);
            }

            // Log if no tools were found (learning opportunity)
            if expanded_tools.is_empty() {
                tracing::warn!(
                    target: "arkavo_tools::keyword_miss",
                    keywords = ?requested_keywords,
                    "No tools matched requested keywords - potential alias candidates"
                );
            }

            // Feed back the tool definitions to the LLM
            let tool_list = expanded_tools
                .iter()
                .map(|t| {
                    format!(
                        "- {}: {}",
                        t.name,
                        t.description.as_deref().unwrap_or("No description")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let tool_response = if expanded_tools.is_empty() {
                "No matching tools found for the requested keywords. Available tools can be listed with 'list all tools'.".to_string()
            } else {
                format!(
                    "Found {} matching tool(s):\n{}\n\nYou can now use these tools.",
                    expanded_tools.len(),
                    tool_list
                )
            };

            messages.push(Message::assistant(&response.content));
            messages.push(Message::user(&tool_response));
            continue; // Re-route with expanded knowledge (doesn't count as iteration)
        }

        // Now we're doing actual work (executing tools or returning final response)
        // Increment iteration counter and check limit
        iteration += 1;
        if iteration > config.max_tool_iterations {
            return Err(format!(
                "Maximum tool iterations ({}) exceeded",
                config.max_tool_iterations
            )
            .into());
        }

        if response.tool_calls.is_empty() {
            return Ok(ToolIntegrationResult {
                final_response: response.content,
                tool_executions: all_tool_executions,
                total_iterations: iteration,
            });
        }

        // Always show concise tool execution info
        let tool_names: Vec<&str> = response
            .tool_calls
            .iter()
            .map(|tc| tc.tool_name.as_str())
            .collect();
        println!("→ {}", tool_names.join(", "));

        if config.show_tool_execution {
            println!("\n=== Tool Execution (Iteration {iteration}) ===");
            println!("LLM wants to call {} tool(s)", response.tool_calls.len());
        }

        let mut tool_results = Vec::new();

        for tool_call in &response.tool_calls {
            if config.show_tool_execution {
                println!("\nExecuting tool: {}", tool_call.tool_name);
                if let Ok(args_pretty) = serde_json::to_string_pretty(&tool_call.arguments) {
                    println!("Arguments:\n{args_pretty}");
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
                    println!("{result_pretty}");
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

/// Maximum characters per tool result to prevent exceeding LLM token limits
/// Gemini has 1M token limit (~4 chars/token), so 200K chars is ~50K tokens per result
const MAX_TOOL_RESULT_CHARS: usize = 200_000;

#[cfg(all(unix, feature = "mcp-tools"))]
fn format_tool_results(results: &[ToolExecutionResult]) -> String {
    use std::fmt::Write;

    let mut formatted = String::from("Tool execution results:\n\n");

    for result in results {
        let _ = writeln!(formatted, "Tool: {}", result.tool_name);
        if result.success {
            let result_json =
                serde_json::to_string_pretty(&result.result).unwrap_or_else(|_| "{}".to_string());

            // Truncate large results to prevent exceeding LLM token limits
            if result_json.len() > MAX_TOOL_RESULT_CHARS {
                let truncated = &result_json[..MAX_TOOL_RESULT_CHARS];
                // Find a good break point (newline or space)
                let break_point = truncated
                    .rfind('\n')
                    .or_else(|| truncated.rfind(' '))
                    .unwrap_or(MAX_TOOL_RESULT_CHARS);
                let _ = writeln!(
                    formatted,
                    "Result (truncated from {} to {} chars):\n{}...\n[OUTPUT TRUNCATED - result too large for LLM context]",
                    result_json.len(),
                    break_point,
                    &result_json[..break_point]
                );
            } else {
                let _ = writeln!(formatted, "Result: {result_json}");
            }
        } else {
            let error_msg = result.error.as_deref().unwrap_or("Unknown error");
            let _ = writeln!(formatted, "Error: {error_msg}");
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
        max_tool_iterations: 10,
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
        max_tool_iterations: 10,
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
