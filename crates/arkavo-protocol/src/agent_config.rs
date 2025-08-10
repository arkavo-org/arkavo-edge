use std::collections::HashMap;

/// Agent configuration parsed from AGENTS.md
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub purpose: String,
    pub model: String,
    pub listen: String,
    pub mdns_enabled: bool,
    pub mcp_servers: Vec<McpServerConfig>,
    pub api_keys: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}

/// Parse AGENTS.md configuration content
pub fn parse_agents_config(content: &str) -> Result<Vec<AgentConfig>, Box<dyn std::error::Error>> {
    let mut agents = Vec::new();
    let mut current_agent: Option<AgentConfig> = None;
    let mut in_agent_section = false;
    let mut in_mcp_section = false;
    let mut current_mcp_server: Option<McpServerConfig> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for agent section header
        if trimmed.starts_with("## ") {
            // Save any pending MCP server before switching agents
            if let Some(server) = current_mcp_server.take()
                && let Some(agent) = current_agent.as_mut()
            {
                agent.mcp_servers.push(server);
            }

            // Save previous agent if exists
            if let Some(agent) = current_agent.take() {
                agents.push(agent);
            }

            // Reset MCP section flag
            in_mcp_section = false;

            let name = trimmed.strip_prefix("## ").unwrap_or("").trim().to_string();
            current_agent = Some(AgentConfig {
                name,
                purpose: String::new(),
                model: String::new(),
                listen: String::new(),
                mdns_enabled: true, // Default to true for zero-config
                mcp_servers: Vec::new(),
                api_keys: HashMap::new(),
            });
            in_agent_section = true;
            continue;
        }

        // Skip if not in agent section
        if !in_agent_section || current_agent.is_none() {
            continue;
        }

        // Check for mcp_servers section
        if trimmed == "mcp_servers:" {
            in_mcp_section = true;
            continue;
        }

        // Handle MCP server entries
        if in_mcp_section && trimmed.starts_with("- name:") {
            // Save previous MCP server if exists
            if let Some(server) = current_mcp_server.take()
                && let Some(agent) = current_agent.as_mut()
            {
                agent.mcp_servers.push(server);
            }

            // Start new MCP server
            current_mcp_server = Some(McpServerConfig {
                name: trimmed
                    .strip_prefix("- name:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                command: None,
                args: Vec::new(),
                url: None,
            });
            continue;
        }

        // Parse MCP server properties
        if in_mcp_section
            && current_mcp_server.is_some()
            && let Some(server) = current_mcp_server.as_mut()
        {
            if trimmed.starts_with("command:") {
                server.command = Some(
                    trimmed
                        .strip_prefix("command:")
                        .unwrap_or("")
                        .trim()
                        .trim_matches('"')
                        .to_string(),
                );
            } else if trimmed.starts_with("args:") {
                // Parse array format: ["arg1", "arg2"]
                let args_str = trimmed.strip_prefix("args:").unwrap_or("").trim();
                if args_str.starts_with('[') && args_str.ends_with(']') {
                    let args_content = &args_str[1..args_str.len() - 1];
                    server.args = args_content
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            } else if trimmed.starts_with("url:") {
                server.url = Some(
                    trimmed
                        .strip_prefix("url:")
                        .unwrap_or("")
                        .trim()
                        .trim_matches('"')
                        .to_string(),
                );
            } else if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('-')
            {
                // End of MCP section
                in_mcp_section = false;
                if let Some(server) = current_mcp_server.take()
                    && let Some(agent) = current_agent.as_mut()
                {
                    agent.mcp_servers.push(server);
                }
            }
        }

        // Parse agent properties (when not in MCP section)
        if !in_mcp_section && let Some(agent) = current_agent.as_mut() {
            if trimmed.starts_with("purpose:") {
                agent.purpose = trimmed
                    .strip_prefix("purpose:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            } else if trimmed.starts_with("model:") {
                agent.model = trimmed
                    .strip_prefix("model:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            } else if trimmed.starts_with("listen:") {
                agent.listen = trimmed
                    .strip_prefix("listen:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            } else if trimmed.starts_with("mdns:") {
                // Only disable if explicitly set to false
                agent.mdns_enabled = !trimmed.contains("false");
            } else if trimmed.contains("_API_KEY:") || trimmed.contains("_api_key:") {
                // Parse API key entries (e.g., MOONSHOT_API_KEY: sk-xxx)
                if let Some(colon_pos) = trimmed.find(':') {
                    let key_name = trimmed[..colon_pos].trim().to_string();
                    let key_value = trimmed[colon_pos + 1..]
                        .trim()
                        .trim_matches('"')
                        .to_string();
                    agent.api_keys.insert(key_name, key_value);
                }
            }
        }
    }

    // Save any pending MCP server
    if let Some(server) = current_mcp_server
        && let Some(agent) = current_agent.as_mut()
    {
        agent.mcp_servers.push(server);
    }

    // Save last agent
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}
