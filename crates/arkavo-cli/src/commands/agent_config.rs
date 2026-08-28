//! `AGENTS.md` configuration parsing: `AgentConfig`, `McpServerConfig`, and the
//! front-matter / markdown parsers that populate them.

/// Default entitlements requested for a delegated agent (attribute FQNs, the
/// vocabulary authnz-rs delegates from the human's stored entitlements).
pub const DEFAULT_AGENT_ENTITLEMENTS: &[&str] = &[
    "https://arkavo.ai/attr/tdf/value/decrypt",
    "https://arkavo.ai/attr/action/value/read",
];

fn default_entitlements() -> Vec<String> {
    DEFAULT_AGENT_ENTITLEMENTS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// Agent configuration parsing
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub purpose: String, // Used as system prompt for LLM
    pub model: String,
    pub mode: arkavo_protocol::agent_config::AgentMode,
    pub listen: String,
    pub mdns_enabled: bool,
    pub mcp_servers: Vec<McpServerConfig>,
    pub api_keys: std::collections::HashMap<String, String>,
    pub quiet: bool, // Default true (quiet), false if --verbose is specified
    // A2A peer configuration
    pub peers: Vec<String>,               // e.g., ["http://localhost:8352"]
    pub a2a_enabled: bool,                // Default: true
    pub a2a_service_type: Option<String>, // Custom mDNS service type
    pub swarm: Option<String>,            // Domain/swarm identifier for learning isolation
    // Attribute FQNs requested for this agent's delegated identity (see
    // DEFAULT_AGENT_ENTITLEMENTS); the human approves this list in the app.
    pub entitlements: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}

pub fn parse_agents_config(content: &str) -> Result<Vec<AgentConfig>, Box<dyn std::error::Error>> {
    let mut agents = Vec::new();
    let mut current_agent: Option<AgentConfig> = None;
    let mut in_agent_section = false;
    let mut in_mcp_section = false;
    let mut current_mcp_server: Option<McpServerConfig> = None;
    let mut current_section: Option<String> = None;
    let mut in_peers_section = false;
    let mut in_entitlements_section = false;
    let mut in_args_section = false;
    let mut in_a2a_section = false;
    let mut in_purpose_multiline = false;
    let mut purpose_lines: Vec<String> = Vec::new();

    // Handle YAML frontmatter format (content between --- delimiters)
    if let Some(after_open) = content.strip_prefix("---")
        && let Some(end_idx) = after_open.find("\n---")
    {
        let frontmatter = &after_open[..end_idx];
        let mut agent = AgentConfig {
            name: String::new(),
            purpose: String::new(),
            model: String::new(),
            mode: arkavo_protocol::agent_config::AgentMode::default(),
            listen: "0.0.0.0:0".to_string(),
            mdns_enabled: true,
            mcp_servers: Vec::new(),
            api_keys: std::collections::HashMap::new(),
            quiet: true,
            peers: Vec::new(),
            a2a_enabled: true,
            a2a_service_type: None,
            swarm: None,
            entitlements: default_entitlements(),
        };
        // Known top-level YAML keys that parse_yaml_properties handles
        const KNOWN_SECTIONS: &[&str] = &[
            "name:",
            "purpose:",
            "model:",
            "mode:",
            "listen:",
            "mdns:",
            "swarm:",
            "a2a:",
            "peers:",
            "entitlements:",
            "mcp_servers:",
            "discovery:",
        ];
        let mut in_unknown_section = false;
        for line in frontmatter.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Non-indented line: check if it starts a known or unknown section
            if !line.starts_with(' ') && !line.starts_with('\t') {
                let is_known = KNOWN_SECTIONS.iter().any(|k| trimmed.starts_with(k))
                    || trimmed.contains("_API_KEY:")
                    || trimmed.contains("_api_key:");
                in_unknown_section = !is_known;
            }
            if in_unknown_section {
                continue;
            }
            parse_yaml_properties(
                line,
                trimmed,
                &mut agent,
                &mut in_mcp_section,
                &mut current_mcp_server,
                &mut in_peers_section,
                &mut in_entitlements_section,
                &mut in_a2a_section,
                &mut in_args_section,
                &mut in_purpose_multiline,
                &mut purpose_lines,
            );
        }
        if in_purpose_multiline {
            agent.purpose = purpose_lines.join("\n").trim().to_string();
        }
        if let Some(server) = current_mcp_server.take() {
            agent.mcp_servers.push(server);
        }
        return Ok(vec![agent]);
    }

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
                    purpose: String::new(),
                    model: String::new(),
                    mode: arkavo_protocol::agent_config::AgentMode::default(),
                    listen: "0.0.0.0:0".to_string(),
                    mdns_enabled: true,
                    mcp_servers: Vec::new(),
                    api_keys: std::collections::HashMap::new(),
                    quiet: true, // Default is quiet
                    peers: Vec::new(),
                    a2a_enabled: true,
                    a2a_service_type: None,
                    swarm: None,
                    entitlements: default_entitlements(),
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
                        purpose: String::new(),
                        model: String::new(),
                        mode: arkavo_protocol::agent_config::AgentMode::default(),
                        listen: "0.0.0.0:0".to_string(),
                        mdns_enabled: true,
                        mcp_servers: Vec::new(),
                        api_keys: std::collections::HashMap::new(),
                        quiet: true, // Default is quiet
                        peers: Vec::new(),
                        a2a_enabled: true,
                        a2a_service_type: None,
                        swarm: None,
                        entitlements: default_entitlements(),
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
                    &mut in_entitlements_section,
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
                    purpose: String::new(),
                    model: String::new(),
                    mode: arkavo_protocol::agent_config::AgentMode::default(),
                    listen: "0.0.0.0:0".to_string(), // Dynamic port
                    mdns_enabled: true,              // Default to true for zero-config
                    mcp_servers: Vec::new(),
                    api_keys: std::collections::HashMap::new(),
                    quiet: true, // Default is quiet
                    peers: Vec::new(),
                    a2a_enabled: true,
                    a2a_service_type: None,
                    swarm: None,
                    entitlements: default_entitlements(),
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
                    &mut in_entitlements_section,
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

    Ok(agents)
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

// Helper function to parse YAML-style properties (used by both old and new format parsers)
#[allow(clippy::too_many_arguments)]
fn parse_yaml_properties(
    line: &str,
    trimmed: &str,
    agent: &mut AgentConfig,
    in_mcp_section: &mut bool,
    current_mcp_server: &mut Option<McpServerConfig>,
    in_peers_section: &mut bool,
    in_entitlements_section: &mut bool,
    in_a2a_section: &mut bool,
    in_args_section: &mut bool,
    in_purpose_multiline: &mut bool,
    purpose_lines: &mut Vec<String>,
) {
    // Check for a2a section
    if trimmed == "a2a:" {
        *in_a2a_section = true;
        return;
    }

    // Check for peers section (can be top-level or under a2a)
    if trimmed == "peers:" {
        *in_peers_section = true;
        return;
    }

    // Handle peer entries
    if *in_peers_section && trimmed.starts_with("- ") {
        let peer = trimmed
            .strip_prefix("- ")
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_string();
        if !peer.is_empty() {
            agent.peers.push(peer);
        }
        return;
    }

    // End peers section when we hit a non-list item
    if *in_peers_section
        && !trimmed.is_empty()
        && !trimmed.starts_with('-')
        && !trimmed.starts_with(' ')
    {
        *in_peers_section = false;
    }

    // Check for entitlements section (attribute FQNs requested for the agent's
    // delegated identity). An explicit key replaces the built-in default list.
    if trimmed == "entitlements:" {
        *in_entitlements_section = true;
        agent.entitlements.clear();
        return;
    }

    // Handle entitlement entries
    if *in_entitlements_section && trimmed.starts_with("- ") {
        let entitlement = trimmed
            .strip_prefix("- ")
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_string();
        if !entitlement.is_empty() {
            agent.entitlements.push(entitlement);
        }
        return;
    }

    // End entitlements section when we hit a non-list item
    if *in_entitlements_section
        && !trimmed.is_empty()
        && !trimmed.starts_with('-')
        && !trimmed.starts_with(' ')
    {
        *in_entitlements_section = false;
    }

    // Parse a2a properties
    if *in_a2a_section && !*in_peers_section {
        if trimmed.starts_with("enabled:") {
            agent.a2a_enabled = !trimmed.contains("false");
            return;
        } else if trimmed.starts_with("service_type:") {
            agent.a2a_service_type = Some(
                trimmed
                    .strip_prefix("service_type:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
            return;
        }
        // End a2a section when we hit a non-indented, non-a2a property
        if !trimmed.is_empty()
            && !trimmed.starts_with(' ')
            && !trimmed.starts_with('-')
            && !trimmed.starts_with("enabled:")
            && !trimmed.starts_with("service_type:")
            && trimmed != "peers:"
            && !trimmed.starts_with("discovery:")
        {
            *in_a2a_section = false;
        }
    }

    // Check for mcp_servers section
    if trimmed == "mcp_servers:" {
        *in_mcp_section = true;
        *in_a2a_section = false;
        *in_peers_section = false;
        return;
    }

    // Handle MCP server entries
    if *in_mcp_section && trimmed.starts_with("- name:") {
        // Save previous MCP server if exists
        if let Some(server) = current_mcp_server.take() {
            agent.mcp_servers.push(server);
        }

        // Start new MCP server
        *current_mcp_server = Some(McpServerConfig {
            name: trimmed
                .strip_prefix("- name:")
                .unwrap_or("")
                .trim()
                .to_string(),
            command: None,
            args: Vec::new(),
            url: None,
        });
        return;
    }

    // Handle YAML list args (e.g., "- value")
    if *in_args_section && trimmed.starts_with("- ") {
        if let Some(server) = current_mcp_server.as_mut() {
            let arg = trimmed
                .strip_prefix("- ")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
            if !arg.is_empty() {
                server.args.push(arg);
            }
        }
        return;
    }

    // End args section when we hit a non-list item
    if *in_args_section && !trimmed.starts_with("- ") && !trimmed.is_empty() {
        *in_args_section = false;
    }

    // Parse MCP server properties
    if *in_mcp_section && let Some(server) = current_mcp_server.as_mut() {
        if trimmed.starts_with("command:") {
            *in_args_section = false;
            server.command = Some(
                trimmed
                    .strip_prefix("command:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
        } else if trimmed.starts_with("args:") {
            // Check for inline array format: ["arg1", "arg2"]
            let args_str = trimmed.strip_prefix("args:").unwrap_or("").trim();
            if args_str.starts_with('[') && args_str.ends_with(']') {
                let args_content = &args_str[1..args_str.len() - 1];
                server.args = args_content
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            } else if args_str.is_empty() {
                // Start YAML list mode for args
                *in_args_section = true;
            }
        } else if trimmed.starts_with("url:") {
            *in_args_section = false;
            server.url = Some(
                trimmed
                    .strip_prefix("url:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
        } else if trimmed.starts_with("transport:") {
            *in_args_section = false;
            // Skip transport for now, not used
        } else if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('-') {
            // End of MCP section
            *in_mcp_section = false;
            *in_args_section = false;
            if let Some(server) = current_mcp_server.take() {
                agent.mcp_servers.push(server);
            }
        }
    }

    // Parse agent properties (when not in MCP section)
    if !*in_mcp_section {
        if trimmed.starts_with("name:") && !trimmed.starts_with("name: |") {
            agent.name = trimmed
                .strip_prefix("name:")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if trimmed.starts_with("purpose:") {
            let value = trimmed.strip_prefix("purpose:").unwrap_or("").trim();
            if value == "|" || value == "|-" || value == "|+" {
                // Start multi-line YAML string
                *in_purpose_multiline = true;
                purpose_lines.clear();
            } else {
                // Single-line purpose
                agent.purpose = value.trim_matches('"').to_string();
            }
        } else if *in_purpose_multiline {
            // Collect multi-line purpose content
            if line.starts_with("  ") || line.starts_with('\t') {
                // Indented line - part of multi-line value
                purpose_lines.push(line.trim().to_string());
            } else if trimmed.is_empty() {
                // Empty line in multi-line - preserve as paragraph break
                purpose_lines.push(String::new());
            } else {
                // Non-indented, non-empty line - end of multi-line
                *in_purpose_multiline = false;
                agent.purpose = purpose_lines.join("\n").trim().to_string();
                purpose_lines.clear();
                // Re-process this line (it might be a new property)
                // Continue to let other parsers handle it
            }
        }

        // End multi-line purpose on new property/section
        if *in_purpose_multiline && (trimmed.starts_with("model:") || trimmed.starts_with('#')) {
            *in_purpose_multiline = false;
            agent.purpose = purpose_lines.join("\n").trim().to_string();
            purpose_lines.clear();
        }

        if trimmed.starts_with("model:") {
            agent.model = trimmed
                .strip_prefix("model:")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if trimmed.starts_with("mode:") {
            let mode_str = trimmed
                .strip_prefix("mode:")
                .unwrap_or("")
                .trim()
                .trim_matches('"');
            agent.mode = match mode_str {
                "specialist" => arkavo_protocol::agent_config::AgentMode::Specialist,
                _ => arkavo_protocol::agent_config::AgentMode::Orchestrator,
            };
        } else if trimmed.starts_with("listen:") {
            agent.listen = trimmed
                .strip_prefix("listen:")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if trimmed.starts_with("swarm:") {
            agent.swarm = Some(
                trimmed
                    .strip_prefix("swarm:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
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
        // Note: autonomous_interval and event_tool are deprecated - push notifications are used instead
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_basic() {
        let content = "---\nname: my-agent\npurpose: \"Test agent\"\nmodel: ministral-3b\n---\n\n# My Agent\nSome docs here.\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "my-agent");
        assert_eq!(agents[0].purpose, "Test agent");
        assert_eq!(agents[0].model, "ministral-3b");
    }

    #[test]
    fn parse_frontmatter_with_a2a() {
        let content = "---\nname: bridge-agent\npurpose: \"Bridge\"\nmodel: ministral-3b\n\na2a:\n  enabled: true\n  service_type: \"bridge\"\n---\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "bridge-agent");
        assert!(agents[0].a2a_enabled);
        assert_eq!(agents[0].a2a_service_type.as_deref(), Some("bridge"));
    }

    #[test]
    fn parse_frontmatter_with_yaml_comments() {
        let content = "---\nname: kas-agent\npurpose: \"KAS demo\"\nmodel: ministral-3b\n\n# This is a YAML comment\n# Another comment\n\na2a:\n  enabled: true\n---\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "kas-agent");
        assert_eq!(agents[0].purpose, "KAS demo");
        assert!(agents[0].a2a_enabled);
    }

    #[test]
    fn parse_frontmatter_openclaw_bridge() {
        let content = r#"---
name: arkavo-bridge-agent
purpose: "A2A protocol bridge demonstrating TDF encryption, budget enforcement, and preflight policies"
model: ministral-3b

kas:
  enabled: true
  key_id: "bridge-demo-key-1"
  algorithm: "ec:secp256r1"
  trusted_roots:
    - did: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
      name: "Demo Root Authority"

a2a:
  enabled: true
  discovery:
    mdns: true
---

# Arkavo Bridge Agent

This agent serves as Arkavo's side of the A2A protocol bridge.
"#;
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "arkavo-bridge-agent");
        assert_eq!(agents[0].model, "ministral-3b");
        assert!(agents[0].a2a_enabled);
        assert!(agents[0].purpose.contains("A2A protocol bridge"));
    }

    #[test]
    fn parse_frontmatter_unknown_section_name_not_leaked() {
        // Regression: nested `name:` inside kas: trusted_roots must not override agent name
        let content = "---\nname: my-agent\npurpose: \"Test\"\nmodel: test\nkas:\n  trusted_roots:\n    - did: \"did:key:abc\"\n      name: \"Root Authority\"\n---\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "my-agent");
    }

    #[test]
    fn parse_frontmatter_no_closing_delimiter_falls_through() {
        let content =
            "---\nname: broken\n\n## actual-agent\npurpose: \"works\"\nmodel: ministral-3b\n";
        let agents = parse_agents_config(content).unwrap();
        // No closing ---, so frontmatter path is skipped; falls through to existing parser
        assert!(!agents.is_empty());
        assert_eq!(agents[0].name, "actual-agent");
        assert_eq!(agents[0].purpose, "works");
    }

    #[test]
    fn parse_existing_format_still_works() {
        let content = "## my-agent\nname: my-agent\npurpose: \"Test\"\nmodel: ministral-3b\nlisten: 0.0.0.0:8080\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "my-agent");
        assert_eq!(agents[0].purpose, "Test");
        assert_eq!(agents[0].model, "ministral-3b");
        assert_eq!(agents[0].listen, "0.0.0.0:8080");
    }

    #[test]
    fn parse_frontmatter_with_peers() {
        let content = "---\nname: peer-agent\npurpose: \"Peer test\"\nmodel: ministral-3b\npeers:\n  - \"localhost:8081\"\n  - \"localhost:8082\"\n---\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].peers.len(), 2);
        assert_eq!(agents[0].peers[0], "localhost:8081");
        assert_eq!(agents[0].peers[1], "localhost:8082");
    }

    #[test]
    fn parse_frontmatter_with_mcp_servers() {
        let content = "---\nname: mcp-agent\npurpose: \"MCP test\"\nmodel: ministral-3b\nmcp_servers:\n  - name: my-server\n    command: npx\n    args: [\"arg1\", \"arg2\"]\n---\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].mcp_servers.len(), 1);
        assert_eq!(agents[0].mcp_servers[0].name, "my-server");
        assert_eq!(agents[0].mcp_servers[0].command.as_deref(), Some("npx"));
        assert_eq!(agents[0].mcp_servers[0].args, vec!["arg1", "arg2"]);
    }

    #[test]
    fn front_matter_entitlements_parsed_and_defaulted() {
        let with = "---\nname: a\npurpose: p\nmodel: m\nentitlements:\n  - https://arkavo.ai/attr/tdf/value/decrypt\n---\n";
        let cfg = parse_agents_config(with).unwrap();
        assert_eq!(
            cfg[0].entitlements,
            vec!["https://arkavo.ai/attr/tdf/value/decrypt"]
        );

        let without = "---\nname: a\npurpose: p\nmodel: m\n---\n";
        let cfg = parse_agents_config(without).unwrap();
        assert_eq!(cfg[0].entitlements, DEFAULT_AGENT_ENTITLEMENTS);
    }
}
