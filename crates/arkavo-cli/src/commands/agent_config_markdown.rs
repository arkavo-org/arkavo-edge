//! Markdown-body half of AGENTS.md parsing: `## agent-name` sections plus the older
//! `# AGENTS.md — name` and `## Agent Identity` shapes.

use super::agent_config::{AgentConfig, McpServerConfig};
use super::agent_config_yaml::parse_yaml_properties;

/// Parse every agent section of a markdown body, in file order.
///
/// A heading with no `key: value` lines still yields a config: the caller decides
/// whether a bare heading names an agent or is just prose.
pub(crate) fn parse_markdown_sections(content: &str) -> Vec<AgentConfig> {
    let mut agents = Vec::new();
    let mut current_agent: Option<AgentConfig> = None;
    let mut in_agent_section = false;
    let mut in_mcp_section = false;
    let mut current_mcp_server: Option<McpServerConfig> = None;
    let mut current_section: Option<String> = None;
    let mut in_peers_section = false;
    let mut in_args_section = false;
    let mut in_a2a_section = false;
    let mut in_purpose_multiline = false;
    let mut purpose_lines: Vec<String> = Vec::new();

    // Check if this is the new markdown format by looking for specific patterns
    let is_new_format = content.contains("## Agent Identity")
        || content.contains("**Mission:**")
        || content.contains("**Name:**")
        || content.contains("# AGENTS.md");

    for line in content.lines() {
        let trimmed = line.trim();

        // Handle new markdown format
        if is_new_format {
            // Extract agent name from H1 title (e.g., "# AGENTS.md — alert-manager")
            if trimmed.starts_with("# AGENTS.md") && trimmed.contains("—") {
                // Save previous agent if exists
                if let Some(agent) = current_agent.take() {
                    agents.push(agent);
                }

                let name = if let Some(em_dash_pos) = trimmed.find("—") {
                    trimmed[em_dash_pos + "—".len()..].trim().to_string()
                } else if let Some(dash_pos) = trimmed.find(" - ") {
                    trimmed[dash_pos + 3..].trim().to_string()
                } else {
                    "unnamed-agent".to_string()
                };

                current_agent = Some(AgentConfig {
                    name,
                    ..AgentConfig::default()
                });
                in_agent_section = true;
                continue;
            }

            // Track current markdown section
            if trimmed.starts_with("## ") {
                let section_name = trimmed.strip_prefix("## ").unwrap_or("").to_string();

                // Check if this is an agent definition (not a standard section)
                let is_standard_section = section_name.starts_with("Agent Identity")
                    || section_name.starts_with("Runtime Configuration")
                    || section_name.starts_with("Capabilities")
                    || section_name.starts_with("Tool Requirements")
                    || section_name.starts_with("MCP Server")
                    || section_name.starts_with("Purpose")
                    || section_name.starts_with("Model Configuration")
                    || section_name.starts_with("Rover Configuration")
                    || section_name.starts_with("A2A Protocol")
                    || section_name.starts_with("Logging");

                if !is_standard_section {
                    // Save any pending MCP server before switching agents
                    if let (Some(server), Some(agent)) =
                        (current_mcp_server.take(), current_agent.as_mut())
                    {
                        agent.mcp_servers.push(server);
                    }

                    // Save previous agent if exists
                    if let Some(agent) = current_agent.take() {
                        agents.push(agent);
                    }

                    // Reset MCP section flag
                    in_mcp_section = false;

                    // Create new agent from ## agent-name header
                    current_agent = Some(AgentConfig {
                        name: section_name.clone(),
                        ..AgentConfig::default()
                    });
                    in_agent_section = true;
                }

                current_section = Some(section_name);
                continue;
            }

            // Parse agent information from markdown sections
            if let Some(agent) = current_agent.as_mut() {
                if let Some(section) = &current_section {
                    match section.as_str() {
                        "Agent Identity" => {
                            // Extract name
                            if trimmed.starts_with("- **Name:**") {
                                if let Some(name) =
                                    extract_markdown_field_value(trimmed, "**Name:**")
                                {
                                    agent.name = name;
                                }
                            }
                            // Extract mission/purpose
                            else if trimmed.starts_with("- **Mission:**")
                                && let Some(mission) =
                                    extract_markdown_field_value(trimmed, "**Mission:**")
                            {
                                agent.purpose = mission;
                            }
                        }
                        "Runtime Configuration (example)" => {
                            // Try to extract listen address from YAML block
                            if trimmed.starts_with("listen:")
                                && let Some(listen) = extract_yaml_value(trimmed, "listen:")
                            {
                                agent.listen = listen;
                            }
                        }
                        _ => {}
                    }
                }

                // Also handle direct YAML-style properties in markdown (for flexibility)
                parse_yaml_properties(
                    line,
                    trimmed,
                    agent,
                    &mut in_mcp_section,
                    &mut current_mcp_server,
                    &mut in_peers_section,
                    &mut in_a2a_section,
                    &mut in_args_section,
                    &mut in_purpose_multiline,
                    &mut purpose_lines,
                );
            }
        } else {
            // Handle old YAML-style format
            // Check for single # header (creates new agent)
            let is_top_level_header = trimmed.starts_with("# ") && !trimmed.starts_with("## ");

            // Check for ## header that's not a standard section
            let is_new_agent_section = if trimmed.starts_with("## ") {
                let section_name = trimmed.strip_prefix("## ").unwrap_or("").trim();
                // Standard sections don't create new agents
                !section_name.starts_with("Agent Identity")
                    && !section_name.starts_with("Runtime Configuration")
                    && !section_name.starts_with("Capabilities")
                    && !section_name.starts_with("Tool Requirements")
                    && !section_name.starts_with("MCP Server")
                    && !section_name.starts_with("Purpose")
                    && !section_name.starts_with("Model Configuration")
                    && !section_name.starts_with("Rover Configuration")
                    && !section_name.starts_with("A2A Protocol")
                    && !section_name.starts_with("Logging")
            } else {
                false
            };

            let is_agent_header = is_top_level_header || is_new_agent_section;

            if is_agent_header {
                // Save any pending MCP server before switching agents
                if let (Some(server), Some(agent)) =
                    (current_mcp_server.take(), current_agent.as_mut())
                {
                    agent.mcp_servers.push(server);
                }

                // Save previous agent if exists
                if let Some(agent) = current_agent.take() {
                    agents.push(agent);
                }

                // Reset MCP section flag
                in_mcp_section = false;

                // Extract name from header
                let header_prefix = if trimmed.starts_with("## ") {
                    "## "
                } else {
                    "# "
                };
                let header_text = trimmed.strip_prefix(header_prefix).unwrap_or("").trim();

                // Use a default name that will be overridden by explicit name: field
                let name = header_text.to_string();

                current_agent = Some(AgentConfig {
                    name,
                    ..AgentConfig::default()
                });
                in_agent_section = true;
                continue;
            }

            // Skip if not in agent section
            if !in_agent_section || current_agent.is_none() {
                continue;
            }

            if let Some(agent) = current_agent.as_mut() {
                parse_yaml_properties(
                    line,
                    trimmed,
                    agent,
                    &mut in_mcp_section,
                    &mut current_mcp_server,
                    &mut in_peers_section,
                    &mut in_a2a_section,
                    &mut in_args_section,
                    &mut in_purpose_multiline,
                    &mut purpose_lines,
                );
            }
        }
    }

    // Save any pending MCP server
    if let (Some(server), Some(agent)) = (current_mcp_server.take(), current_agent.as_mut()) {
        agent.mcp_servers.push(server);
    }

    // Save last agent
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    agents
}

// Helper function to extract value from markdown field like "- **Name:** value"
fn extract_markdown_field_value(line: &str, field_prefix: &str) -> Option<String> {
    if let Some(start_pos) = line.find(field_prefix) {
        let after_prefix = &line[start_pos + field_prefix.len()..];
        let value = after_prefix.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

// Helper function to extract YAML-style values like "listen: 0.0.0.0:8342"
fn extract_yaml_value(line: &str, key: &str) -> Option<String> {
    if let Some(colon_pos) = line.find(':') {
        let key_part = line[..colon_pos].trim();
        if key_part == key.trim_end_matches(':') {
            let value = line[colon_pos + 1..].trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
