//! `AGENTS.md` configuration domain model.
//!
//! `AgentConfig`, `McpServerConfig`, and `parse_agents_config` -- the entry point that walks front-matter / markdown text (via `agent_config_parse`'s helpers) to populate them.

use super::agent_config_parse::{
    extract_markdown_field_value, extract_yaml_value, parse_yaml_properties,
};

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

    // Regression (R21): an inline-array `entitlements:` line previously
    // failed an exact-equality check elsewhere in the parser and was
    // silently dropped, leaving the (broader) default set advertised
    // instead -- an author asking for a narrower grant silently got a wider
    // one. These cover the inline form through the front-matter path.

    #[test]
    fn front_matter_entitlements_inline_array_parsed() {
        let content = "---\nname: a\npurpose: p\nmodel: m\nentitlements: [\"https://arkavo.ai/attr/tdf/value/decrypt\"]\n---\n";
        let cfg = parse_agents_config(content).unwrap();
        assert_eq!(
            cfg[0].entitlements,
            vec!["https://arkavo.ai/attr/tdf/value/decrypt"],
            "inline array must produce exactly the requested (narrower) list, not the defaults"
        );
    }

    #[test]
    fn front_matter_entitlements_inline_empty_array_parsed() {
        let content = "---\nname: a\npurpose: p\nmodel: m\nentitlements: []\n---\n";
        let cfg = parse_agents_config(content).unwrap();
        assert!(
            cfg[0].entitlements.is_empty(),
            "an explicit empty list must stay empty, not fall back to the defaults"
        );
    }

    // Same inline forms, through the old (non-front-matter) `## agent-name`
    // parsing path, which shares `parse_yaml_properties` with the
    // front-matter path but has no `KNOWN_SECTIONS` gate of its own.

    #[test]
    fn old_format_entitlements_inline_array_parsed() {
        let content = "## my-agent\nname: my-agent\npurpose: \"Test\"\nmodel: ministral-3b\nentitlements: [\"https://arkavo.ai/attr/tdf/value/decrypt\"]\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(
            agents[0].entitlements,
            vec!["https://arkavo.ai/attr/tdf/value/decrypt"]
        );
    }

    #[test]
    fn old_format_entitlements_inline_empty_array_parsed() {
        let content = "## my-agent\nname: my-agent\npurpose: \"Test\"\nmodel: ministral-3b\nentitlements: []\n";
        let agents = parse_agents_config(content).unwrap();
        assert!(agents[0].entitlements.is_empty());
    }

    #[test]
    fn old_format_entitlements_absent_defaults() {
        let content = "## my-agent\nname: my-agent\npurpose: \"Test\"\nmodel: ministral-3b\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents[0].entitlements, DEFAULT_AGENT_ENTITLEMENTS);
    }

    // Same inline form again, through the *new* markdown format (detected by
    // "# AGENTS.md" / "**Mission:**" markers), which also feeds every line
    // through the shared `parse_yaml_properties` helper.
    #[test]
    fn new_markdown_format_entitlements_inline_array_parsed() {
        let content = "# AGENTS.md — my-agent\n\n- **Mission:** test\nname: my-agent\npurpose: \"Test\"\nmodel: ministral-3b\nentitlements: [\"https://arkavo.ai/attr/tdf/value/decrypt\"]\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(
            agents[0].entitlements,
            vec!["https://arkavo.ai/attr/tdf/value/decrypt"]
        );
    }
}
