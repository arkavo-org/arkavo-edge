#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_llm::{Message, ParsedToolCall, ProviderResponse, ToolExecutionResult, ToolExecutor};
#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_mcp_tools::{McpClient as McpClientTrait, ToolRegistry};
#[cfg(all(unix, feature = "mcp-tools"))]
use arkavo_router::Router;
#[cfg(all(unix, feature = "mcp-tools"))]
use std::collections::HashSet;
#[cfg(all(unix, feature = "mcp-tools"))]
use std::collections::hash_map::DefaultHasher;
#[cfg(all(unix, feature = "mcp-tools"))]
use std::hash::{Hash, Hasher};
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

#[cfg(all(unix, feature = "mcp-tools"))]
async fn verify_response_with_critic(
    critic: &arkavo_critic::CriticPipeline,
    task_description: &str,
    response: &ProviderResponse,
    tool_calls: Vec<ParsedToolCall>,
    available_tools: Vec<arkavo_mcp_tools::ToolInfo>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut response_for_critic = response.clone();
    response_for_critic.tool_calls = tool_calls;

    let input = arkavo_critic::VerificationInput::new(
        task_description.to_string(),
        response_for_critic,
        available_tools,
    );
    let result = critic.verify(&input).await;
    if result.passed {
        Ok(())
    } else {
        let failures: Vec<String> = result
            .failures()
            .iter()
            .map(|e| e.description.clone())
            .collect();
        Err(format!("Response rejected by critic: {failures:?}").into())
    }
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

    // Load preflight policies from AGENTS.md in current or parent directories
    let moderator = arkavo_router::preflight::load_policies_from_config().unwrap_or_else(|e| {
        tracing::warn!("Failed to load policies: {e}, using empty moderator");
        arkavo_router::preflight::PreflightModerator::new()
    });

    let router = Arc::new(Router::new().await?.with_preflight(moderator));

    // Create critic pipeline for post-LLM validation
    let critic = arkavo_critic::default_pipeline();

    let storage = Arc::new(arkavo_memory::MemoryStorage::new().await?);
    let mut tool_registry = ToolRegistry::from_mcp_or_default(mcp_client, storage);

    // Register tools from each crate
    arkavo_router::tools::register_tools(&mut tool_registry, router.clone());

    // Register mesh orchestration tools for agent coordination
    let mesh_state = std::sync::Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
    arkavo_mcp_mesh::register_tools(&mut tool_registry, mesh_state);

    // Register HRM orchestration tools (Conductor API)
    let hrm_state = std::sync::Arc::new(tokio::sync::RwLock::new(
        arkavo_hrm::tools::HrmToolsState::new(),
    ));
    arkavo_hrm::tools::register_tools(&mut tool_registry, hrm_state);

    // Register UCP payment tools for AI agent commerce
    // Uses the same budget tracker infrastructure for spending limits
    let ucp_budget_tracker = std::sync::Arc::new(
        arkavo_budget::BudgetTracker::new(arkavo_budget::BudgetConfig::default())
            .await
            .map_err(|e| format!("Failed to create UCP budget tracker: {e}"))?,
    );
    let ucp_state = std::sync::Arc::new(tokio::sync::RwLock::new(arkavo_ucp::UcpState::new(
        ucp_budget_tracker,
    )));
    arkavo_ucp::register_tools(&mut tool_registry, ucp_state);

    // Register Claude Agent SDK tools (entitlement-gated, requires OAuth)
    #[cfg(feature = "claude-agent")]
    {
        use arkavo_mcp_claude::{ClaudeCodeCapability, ClaudeCodeConfig};

        // Try to initialize Claude Code capability using default config
        // The default config checks ANTHROPIC_API_KEY env var to determine auth method
        let claude_config = ClaudeCodeConfig::default();
        tracing::debug!(
            "Claude config enabled: {}, workspace: {:?}",
            claude_config.enabled,
            claude_config.workspace_root
        );

        // Only attempt to register if API key is set or OAuth token is cached
        // This prevents blocking on interactive OAuth flow in non-interactive contexts
        if claude_config.enabled && arkavo_mcp_claude::is_auth_available() {
            let event_writer = std::sync::Arc::new(arkavo_events::EventWriter::new(
                arkavo_events::EventWriterConfig::default(),
            ));
            match ClaudeCodeCapability::new(
                claude_config,
                "arkavo-cli".to_string(),
                event_writer,
                None, // budget_tracker
                None, // auth_client
            ) {
                Ok(capability) => {
                    let capability: std::sync::Arc<ClaudeCodeCapability> =
                        std::sync::Arc::new(capability);
                    // Prepare authentication (should succeed since we checked above)
                    if let Err(e) = capability.prepare().await {
                        tracing::warn!("Claude MCP tools not ready: {e}");
                    } else {
                        arkavo_mcp_claude::register_tools(&mut tool_registry, capability);
                        tracing::info!("Claude MCP tools registered");
                    }
                }
                Err(e) => {
                    tracing::warn!("Claude MCP tools disabled: {e}");
                }
            }
        } else if claude_config.enabled {
            tracing::debug!("Claude MCP tools skipped: no API key or cached OAuth token");
        }
    }

    // Wrap registry in Arc for shared access
    let registry_arc = Arc::new(tool_registry);

    // Create executor with the same registry that has router tools registered
    let tool_executor = ToolExecutor::with_registry(registry_arc.clone());

    let mut all_tool_executions = Vec::new();
    let mut iteration = 0;
    let mut first_call = true;
    // Track tools already disclosed to LLM (only disclose once per context)
    let mut disclosed_tools: HashSet<String> = HashSet::new();
    // Safeguard against infinite loops from tool discovery/metadata requests
    let mut discovery_iterations = 0;
    const MAX_DISCOVERY_ITERATIONS: usize = 5;
    // Overall loop safeguard (discovery + execution combined)
    let mut total_loop_iterations = 0;
    const MAX_TOTAL_LOOP_ITERATIONS: usize = 20;
    // Track repeated same-tool calls to detect loops
    let mut last_tool_call: Option<String> = None;
    let mut same_tool_count = 0;
    const MAX_SAME_TOOL_CALLS: usize = 3;

    // Track repeated content to detect model output loops
    let mut last_content_hash: Option<u64> = None;
    let mut same_content_count = 0;
    const MAX_SAME_CONTENT: usize = 2; // Lower threshold - repeated content is a clear sign of loop

    // Helper to detect format confusion (model generating conversation markers)
    fn detect_format_confusion(content: &str) -> Option<&'static str> {
        // Strip think blocks first to avoid false positives in reasoning
        let content_without_think = {
            let mut result = content.to_string();
            while let Some(start) = result.find("<think>") {
                if let Some(end) = result.find("</think>") {
                    let end_pos = end + "</think>".len();
                    result = format!("{}{}", &result[..start], &result[end_pos..]);
                } else {
                    result = result[..start].to_string();
                    break;
                }
            }
            result
        };

        // Model should NEVER generate "Human:" marker - it's a user turn indicator
        // Check for it appearing after any think block or at start of response
        if content_without_think.contains("Human:") {
            return Some("'Human:' marker in model output - format confusion");
        }

        // Model shouldn't generate "Assistant:" markers in its own output
        if content_without_think.contains("Assistant:") {
            return Some("'Assistant:' marker in output - format confusion");
        }

        // Detect repeated multi-line blocks (looking for repeated code fence patterns)
        // This catches models that output the same instructions/examples repeatedly
        let lines: Vec<&str> = content_without_think.lines().collect();
        if lines.len() >= 6 {
            // Look for repeating patterns of 2-5 lines
            for pattern_len in 2..=5 {
                let mut i = 0;
                while i + pattern_len * 3 <= lines.len() {
                    let pattern: Vec<&str> = lines[i..i + pattern_len].to_vec();
                    let pattern_text = pattern.join("\n").trim().to_string();

                    // Skip empty or very short patterns
                    if pattern_text.len() < 10 {
                        i += 1;
                        continue;
                    }

                    // Count consecutive occurrences
                    let mut count = 1;
                    let mut j = i + pattern_len;
                    while j + pattern_len <= lines.len() {
                        let next_pattern: Vec<&str> = lines[j..j + pattern_len].to_vec();
                        let next_text = next_pattern.join("\n").trim().to_string();
                        if next_text == pattern_text {
                            count += 1;
                            j += pattern_len;
                        } else {
                            break;
                        }
                    }

                    if count >= 3 {
                        return Some("Repeated block pattern detected");
                    }
                    i += 1;
                }
            }
        }

        None
    }

    loop {
        // Absolute safeguard against runaway loops
        total_loop_iterations += 1;
        if total_loop_iterations > MAX_TOTAL_LOOP_ITERATIONS {
            tracing::error!(
                "Absolute loop limit reached: {} iterations (discovery: {}, execution: {})",
                total_loop_iterations,
                discovery_iterations,
                iteration
            );
            return Err(format!(
                "Maximum total loop iterations ({MAX_TOTAL_LOOP_ITERATIONS}) exceeded - breaking out of potential infinite loop"
            ).into());
        }

        // Rate limit iterations to avoid overwhelming Terminal.app's process monitoring
        // (Terminal.app has a bug with rapid subprocess enumeration - macOS 26.2)
        if total_loop_iterations > 1 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

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
                inference_timing: None,
            }
        } else {
            // Subsequent iterations - use Thompson Sampling routing
            match router
                .route_with_tools(task_description, messages.clone(), Some(&registry_arc))
                .await
            {
                Ok(r) => r,
                Err(arkavo_router::Error::MissingToolUse { keywords }) => {
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

                    // Filter out already-disclosed tools (only disclose once per context)
                    let new_tools: Vec<_> = expanded_tools
                        .into_iter()
                        .filter(|t| !disclosed_tools.contains(&t.name))
                        .collect();

                    if new_tools.is_empty() {
                        tracing::warn!(
                            target: "arkavo_tools::judge_keyword_miss",
                            keywords = ?keywords,
                            "Judge suggested keywords but no new tools matched (already disclosed or not found)"
                        );
                    }

                    // Track newly disclosed tools
                    for tool in &new_tools {
                        disclosed_tools.insert(tool.name.clone());
                    }

                    // Feed back the tool definitions to the LLM
                    let tool_list = new_tools
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

                    let tool_response = if new_tools.is_empty() {
                        "No matching tools found for the requested information.".to_string()
                    } else {
                        format!(
                            "Found {} relevant tool(s):\n{}\n\nPlease use these tools to answer the question.",
                            new_tools.len(),
                            tool_list
                        )
                    };

                    messages.push(Message::user(&tool_response));

                    // Safeguard against infinite discovery loops
                    discovery_iterations += 1;
                    if discovery_iterations > MAX_DISCOVERY_ITERATIONS {
                        return Err(format!(
                            "Maximum discovery iterations ({MAX_DISCOVERY_ITERATIONS}) exceeded - possible infinite loop"
                        ).into());
                    }
                    continue; // Re-route with expanded knowledge (doesn't count as iteration)
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        };

        // Check for format confusion (model generating conversation markers)
        if let Some(confusion_reason) = detect_format_confusion(&response.content) {
            tracing::warn!("Format confusion detected: {confusion_reason}");
            return Ok(ToolIntegrationResult {
                final_response: format!(
                    "Loop detected: {confusion_reason}. The model appears confused about the conversation format. Please try rephrasing your request.",
                ),
                tool_executions: all_tool_executions,
                total_iterations: iteration,
            });
        }

        // Check for repeated content (model output loop detection)
        let mut hasher = DefaultHasher::new();
        response.content.hash(&mut hasher);
        let content_hash = hasher.finish();

        if Some(content_hash) == last_content_hash {
            same_content_count += 1;
            if same_content_count >= MAX_SAME_CONTENT {
                tracing::warn!(
                    "Model generated same content {} times in a row, breaking loop",
                    same_content_count + 1
                );
                return Ok(ToolIntegrationResult {
                    final_response: "Loop detected: model is generating repetitive content. Please try rephrasing your request.".to_string(),
                    tool_executions: all_tool_executions,
                    total_iterations: iteration,
                });
            }
        } else {
            same_content_count = 0;
            last_content_hash = Some(content_hash);
        }

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

            // Filter out already-disclosed tools (only disclose once per context)
            let new_tools: Vec<_> = expanded_tools
                .into_iter()
                .filter(|t| !disclosed_tools.contains(&t.name))
                .collect();

            // Log if no NEW tools were found (learning opportunity)
            if new_tools.is_empty() {
                tracing::warn!(
                    target: "arkavo_tools::keyword_miss",
                    keywords = ?requested_keywords,
                    "No new tools matched requested keywords (already disclosed or not found)"
                );
            }

            // Track newly disclosed tools
            for tool in &new_tools {
                disclosed_tools.insert(tool.name.clone());
            }

            // Feed back the tool definitions to the LLM
            let tool_list = new_tools
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

            let tool_response = if new_tools.is_empty() {
                "No matching tools found for the requested keywords. Available tools can be listed with 'list all tools'.".to_string()
            } else {
                format!(
                    "Found {} matching tool(s):\n{}\n\nYou can now use these tools.",
                    new_tools.len(),
                    tool_list
                )
            };

            messages.push(Message::assistant(&response.content));
            messages.push(Message::user(&tool_response));

            // Safeguard against infinite discovery loops
            discovery_iterations += 1;
            if discovery_iterations > MAX_DISCOVERY_ITERATIONS {
                return Err(format!(
                    "Maximum discovery iterations ({MAX_DISCOVERY_ITERATIONS}) exceeded - possible infinite loop"
                )
                .into());
            }
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

        // Check for markdown-formatted tool calls (model outputting tool name in code blocks)
        let mut tool_calls = response.tool_calls.clone();
        if tool_calls.is_empty()
            && let Some(parsed_calls) =
                parse_markdown_tool_calls(&response.content, registry_arc.as_ref())
        {
            tracing::debug!(
                "Parsed {} tool call(s) from markdown format",
                parsed_calls.len()
            );
            tool_calls = parsed_calls;
        }

        // Post-LLM validation via critic pipeline for both action and non-action paths
        // This must run before any tool execution to enforce guardrails consistently.
        verify_response_with_critic(
            &critic,
            task_description,
            &response,
            tool_calls.clone(),
            registry_arc.list_tools(),
        )
        .await?;

        if tool_calls.is_empty() {
            return Ok(ToolIntegrationResult {
                final_response: response.content,
                tool_executions: all_tool_executions,
                total_iterations: iteration,
            });
        }

        // Always show concise tool execution info
        let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.tool_name.as_str()).collect();
        let current_tools = tool_names.join(", ");
        println!("→ {current_tools}");

        // Check for repeated same-tool calls (model stuck in loop)
        if Some(&current_tools) == last_tool_call.as_ref() {
            same_tool_count += 1;
            if same_tool_count >= MAX_SAME_TOOL_CALLS {
                tracing::warn!(
                    "Model called same tool(s) {} times in a row, breaking loop",
                    same_tool_count
                );
                // Return the last tool results as the final response
                // The chat layer will use the display field from the tool results
                return Ok(ToolIntegrationResult {
                    final_response: String::new(), // Empty triggers fallback display
                    tool_executions: all_tool_executions,
                    total_iterations: iteration,
                });
            }
        } else {
            same_tool_count = 1;
            last_tool_call = Some(current_tools);
        }

        if config.show_tool_execution {
            println!("\n=== Tool Execution (Iteration {iteration}) ===");
            println!("LLM wants to call {} tool(s)", tool_calls.len());
        }

        let mut tool_results = Vec::new();

        for tool_call in &tool_calls {
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

            // Learn from tool errors: feed domain-specific failures into the
            // prompt advisor so future prompts include corrections
            if !result.success
                && let Some(ref error_msg) = result.error
                && let Some(model_name) = router.last_routed_model()
            {
                let model_family = Router::detect_model_family(&model_name);
                router.advisor().observe_tool_error(
                    &model_family,
                    &tool_call.tool_name,
                    error_msg,
                    &tool_call.arguments,
                );
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

/// Parse markdown-formatted tool calls from response content
/// Only matches when content is clearly a standalone tool invocation attempt
/// (e.g., just ```get_time_status``` with minimal surrounding text)
#[cfg(all(unix, feature = "mcp-tools"))]
fn parse_markdown_tool_calls(
    content: &str,
    registry: &ToolRegistry,
) -> Option<Vec<ParsedToolCall>> {
    use regex::Regex;

    // Only attempt parsing if content is short (under 100 chars) - clear tool invocation attempt
    // Longer responses are normal text that happens to mention tools
    let trimmed = content.trim();
    if trimmed.len() > 100 {
        return None;
    }

    let mut tool_calls = Vec::new();

    // Pattern 1: Code block with tool name (```tool_name``` or ```tool_name {...}```)
    if let Ok(code_block_re) = Regex::new(r"```(\w+)(?:\s*(\{[^}]*}))?```") {
        for cap in code_block_re.captures_iter(trimmed) {
            if let Some(name_match) = cap.get(1) {
                let name = name_match.as_str();
                if registry.get(name).is_some() {
                    let args = cap
                        .get(2)
                        .and_then(|m| serde_json::from_str(m.as_str()).ok())
                        .unwrap_or_else(|| serde_json::json!({}));
                    tool_calls.push(ParsedToolCall {
                        tool_name: name.to_string(),
                        arguments: args,
                        call_id: None,
                    });
                }
            }
        }
    }

    // Pattern 2: Just tool name on its own line (model outputting tool name without formatting)
    if tool_calls.is_empty() && !trimmed.contains(' ') && trimmed.len() < 50 {
        // Single word that might be a tool name
        if registry.get(trimmed).is_some() {
            tool_calls.push(ParsedToolCall {
                tool_name: trimmed.to_string(),
                arguments: serde_json::json!({}),
                call_id: None,
            });
        }
    }

    if tool_calls.is_empty() {
        None
    } else {
        Some(tool_calls)
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

            // Include schema hint when available (helps LLM retry with correct params)
            if let Some(schema) = &result.schema_hint {
                let _ = writeln!(
                    formatted,
                    "\nTo fix this error, use the correct parameter format:"
                );
                if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
                    let _ = writeln!(formatted, "Description: {desc}");
                }
                if let Some(params) = schema.get("parameters") {
                    // Format parameters in a clear, LLM-friendly way
                    if let Some(props) = params.get("properties") {
                        let _ = writeln!(formatted, "Required parameters:");
                        if let Some(required) = params.get("required").and_then(|v| v.as_array()) {
                            for req in required {
                                if let Some(name) = req.as_str()
                                    && let Some(prop_schema) = props.get(name)
                                {
                                    let prop_type = prop_schema
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("any");
                                    let prop_desc = prop_schema
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let _ = writeln!(
                                        formatted,
                                        "  - {name} ({prop_type}): {prop_desc}"
                                    );
                                }
                            }
                        }
                        // Also show optional parameters if any
                        let required_set: std::collections::HashSet<&str> = params
                            .get("required")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                            .unwrap_or_default();

                        let optional_params: Vec<_> = props
                            .as_object()
                            .map(|obj| {
                                obj.iter()
                                    .filter(|(k, _)| !required_set.contains(k.as_str()))
                                    .collect()
                            })
                            .unwrap_or_default();

                        if !optional_params.is_empty() {
                            let _ = writeln!(formatted, "Optional parameters:");
                            for (name, prop_schema) in optional_params {
                                let prop_type = prop_schema
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("any");
                                let prop_desc = prop_schema
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let _ =
                                    writeln!(formatted, "  - {name} ({prop_type}): {prop_desc}");
                            }
                        }
                    }
                }
            }
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

#[cfg(all(unix, feature = "mcp-tools", test))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn critic_blocks_unsafe_content_before_execution() {
        let critic = arkavo_critic::default_pipeline();
        let response = ProviderResponse {
            content: "Here is an api_key: secret123".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
        };

        let result =
            verify_response_with_critic(&critic, "test", &response, vec![], Vec::new()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn critic_validates_schema_on_tool_call_path() {
        let critic = arkavo_critic::default_pipeline();
        let response = ProviderResponse {
            content: "Calling tool".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
        };
        let tool_calls = vec![ParsedToolCall {
            tool_name: "unknown_tool".to_string(),
            arguments: serde_json::json!({}),
            call_id: None,
        }];

        let result =
            verify_response_with_critic(&critic, "test", &response, tool_calls, Vec::new()).await;
        assert!(result.is_err());
    }
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
