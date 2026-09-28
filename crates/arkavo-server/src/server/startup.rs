use super::conductor::execute_with_conductor;
use super::mcp_bridge::McpBridgeTool;
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::mcp_registry::{McpRegistry, Tool};
use std::sync::Arc;

/// Agent planning state for startup goal creation
#[derive(Debug, Clone, Default)]
pub struct AgentPlan {
    pub goals: Vec<AgentGoal>,
    pub watch_for: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentGoal {
    pub description: String,
    pub priority: u8,
    pub status: GoalStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GoalStatus {
    Active,
    InProgress,
    Completed,
}

/// Run startup planning phase for autonomous agents
///
/// This phase executes when:
/// 1. Purpose is known (system prompt from SwarmKit)
/// 2. MCP tools are registered
/// 3. MCP tools are working (first successful response)
pub async fn run_startup_planning_phase(
    system_prompt: &str,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
) -> AgentPlan {
    use arkavo_mcp_tools::ToolRegistry;

    eprintln!("[Planning] Gathering situational context...");

    // 1. Get available tools first
    let tools = mcp_registry.list_all_tools().await.unwrap_or_default();

    // 1b. Generate parameter aliases for each MCP server (if not already done)
    // Group tools by server and generate aliases
    let mut servers_needing_aliases: std::collections::HashMap<String, Vec<Tool>> =
        std::collections::HashMap::new();
    for tool in &tools {
        if let Some((server, tool_name)) = tool.name.split_once(':')
            && !mcp_registry.has_param_aliases(server).await
        {
            let entry = servers_needing_aliases
                .entry(server.to_string())
                .or_default();
            // Store tool with unprefixed name for alias generation
            let mut unprefixed_tool = tool.clone();
            unprefixed_tool.name = tool_name.to_string();
            entry.push(unprefixed_tool);
        }
    }

    // Generate and store aliases for each server
    for (server_name, server_tools) in servers_needing_aliases {
        let aliases = generate_param_aliases(&server_name, &server_tools, router).await;
        if !aliases.is_empty()
            && let Err(e) = mcp_registry.set_param_aliases(&server_name, aliases).await
        {
            eprintln!("[Aliases] Failed to set aliases for {server_name}: {e}");
        }
    }

    // 2. Gather situational context using available MCP tools
    let mut context_parts = Vec::new();

    // Try common situational tools (only if they exist)
    let situational_patterns = ["position", "inventory", "health", "status"];

    for pattern in &situational_patterns {
        // Find a tool matching this pattern
        if let Some(tool) = tools.iter().find(|t| t.name.contains(pattern)) {
            // Strip server prefix for call (call_tool now handles both formats)
            let tool_name = tool.name.split(':').next_back().unwrap_or(&tool.name);
            if let Ok(result) = mcp_registry
                .call_tool(tool_name, serde_json::json!({}), "startup-planning")
                .await
            {
                if let Some(text) = result.as_str() {
                    context_parts.push(format!("{tool_name}: {text}"));
                } else {
                    context_parts.push(format!("{tool_name}: {result}"));
                }
            }
        }
    }

    let situation = if context_parts.is_empty() {
        "No situational context available".to_string()
    } else {
        context_parts.join("\n")
    };

    // 3. Get tool descriptions for planning
    let tool_descriptions: Vec<String> = tools
        .iter()
        .map(|t| format!("- {}: {}", t.name, t.description))
        .collect();

    // 3. Build planning prompt
    let planning_prompt = format!(
        "{}\n\n\
         ## Current Situation\n{}\n\n\
         ## Available Tools\n{}\n\n\
         ## Task\n\
         Based on your purpose and current situation, create an initial plan:\n\
         1. What are your immediate goals? (List 2-4 goals)\n\
         2. What actions should you take first?\n\
         3. What should you watch for in incoming events?\n\n\
         After stating your plan, execute your first action using the appropriate tools.",
        system_prompt,
        situation,
        tool_descriptions.join("\n")
    );

    eprintln!(
        "[Planning] Planning prompt: {} chars, {} tools available",
        planning_prompt.len(),
        tools.len()
    );

    // 4. Project MCP tools for Router
    let mut tool_registry = ToolRegistry::empty();
    if let Ok(mcp_tools) = mcp_registry.list_all_tools().await {
        for tool in mcp_tools {
            let tool_name = tool.name.clone();
            let bridge = McpBridgeTool::new(mcp_registry.clone(), tool);
            tool_registry.register(&tool_name, Box::new(bridge));
        }
    }

    // 5. Execute planning through conductor (uses larger model for high-confidence tasks)
    let result =
        execute_with_conductor(conductor, router, mcp_registry, planning_prompt, None, None).await;

    match result {
        Ok(response) => {
            eprintln!("[Planning] Got planning response: {} chars", response.len());

            // 6. Extract goals from response
            let goals = extract_goals_from_response(&response);
            eprintln!("[Planning] Extracted {} goals", goals.len());

            AgentPlan {
                goals,
                watch_for: vec!["chat".to_string(), "player".to_string()],
            }
        }
        Err(e) => {
            eprintln!("[Planning] Planning failed: {e}");
            AgentPlan::default()
        }
    }
}

/// Extract goals from LLM planning response
fn extract_goals_from_response(response: &str) -> Vec<AgentGoal> {
    let mut goals = Vec::new();

    // Look for numbered items that look like goals
    for line in response.lines() {
        let trimmed = line.trim();

        // Match patterns like "1. Goal description" or "- Goal description"
        let is_numbered = trimmed
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
            && trimmed.contains(". ");
        let is_bulleted = trimmed.starts_with("- ") || trimmed.starts_with("* ");

        if is_numbered || is_bulleted {
            // Extract the description after the marker
            let description = if is_numbered {
                trimmed
                    .split_once(". ")
                    .map(|(_, desc)| desc.trim())
                    .unwrap_or(trimmed)
            } else {
                trimmed.trim_start_matches(['-', '*', ' ']).trim()
            };

            // Skip if too short or looks like a tool call
            if description.len() > 10
                && !description.contains("```")
                && !description.starts_with('{')
            {
                goals.push(AgentGoal {
                    description: description.to_string(),
                    priority: (goals.len() + 1) as u8,
                    status: GoalStatus::Active,
                });
            }
        }

        // Limit to first 5 goals
        if goals.len() >= 5 {
            break;
        }
    }

    goals
}

/// Generate parameter aliases for MCP tools using the LLM
///
/// Asks the LLM to analyze tool schemas and generate alternative parameter names
/// that might be used by smaller models when calling tools.
#[allow(deprecated)] // route_with_tools bypasses architect mode
async fn generate_param_aliases(
    server_name: &str,
    tools: &[Tool],
    router: &Arc<arkavo_router::Router>,
) -> std::collections::HashMap<String, std::collections::HashMap<String, String>> {
    use std::collections::HashMap;

    if tools.is_empty() {
        return HashMap::new();
    }

    // Build a prompt describing the tools and their required parameters
    let mut tool_descriptions = Vec::new();
    for tool in tools {
        if let Some(schema) = &tool.input_schema
            && let Some(required) = schema.get("required").and_then(|r| r.as_array())
        {
            let required_params: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
            if !required_params.is_empty() {
                tool_descriptions.push(format!(
                    "- {}: required parameters: {}",
                    tool.name,
                    required_params.join(", ")
                ));
            }
        }
    }

    if tool_descriptions.is_empty() {
        return HashMap::new();
    }

    let alias_prompt = format!(
        "List parameter aliases for these MCP tools. Output ONLY lines in format:\n\
         tool.param: alt1, alt2, alt3\n\n\
         Tools:\n{}\n\n\
         Example output:\n\
         search.query: q, search_term, keyword\n\
         create-file.fileName: file, name, path\n\n\
         Output ONLY the alias lines, nothing else:",
        tool_descriptions.join("\n")
    );

    // Use the router to call the LLM
    let messages = vec![arkavo_llm::Message::user(alias_prompt)];

    let response = match router.route_with_tools(&"", messages, None).await {
        Ok(resp) => resp.content,
        Err(e) => {
            eprintln!("[Aliases] Failed to generate aliases: {e}");
            return HashMap::new();
        }
    };

    // Parse the response
    let mut aliases: HashMap<String, HashMap<String, String>> = HashMap::new();

    for line in response.lines() {
        let line = line.trim();

        // Skip empty lines, lines without both . and :, or lines that look like explanations
        if line.is_empty()
            || !line.contains('.')
            || !line.contains(':')
            || line.contains("->")
            || line.len() > 100
        {
            continue;
        }

        // Parse "tool_name.param_name: alt1, alt2, alt3"
        if let Some((tool_param, alts)) = line.split_once(':') {
            let tool_param = tool_param.trim();
            // Ensure tool_param looks like "tool-name.paramName"
            if !tool_param
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                continue;
            }

            if let Some((tool_name, param_name)) = tool_param.split_once('.') {
                let tool_name = tool_name.trim().to_lowercase();
                let param_name = param_name.trim().to_string();

                // Skip if tool name or param name is too long (likely garbage)
                if tool_name.len() > 30 || param_name.len() > 30 {
                    continue;
                }

                let tool_aliases = aliases.entry(tool_name).or_default();

                for alt in alts.split(',') {
                    let alt = alt.trim().to_string();
                    // Only accept simple alphanumeric alternatives
                    if !alt.is_empty()
                        && alt != param_name
                        && alt.len() < 30
                        && alt.chars().all(|c| c.is_alphanumeric() || c == '_')
                    {
                        tool_aliases.insert(alt, param_name.clone());
                    }
                }
            }
        }
    }

    let alias_count: usize = aliases.values().map(|m| m.len()).sum();
    if alias_count > 0 {
        eprintln!(
            "[Aliases] Generated {} aliases for {} tools on server {}",
            alias_count,
            aliases.len(),
            server_name
        );
    }

    aliases
}
