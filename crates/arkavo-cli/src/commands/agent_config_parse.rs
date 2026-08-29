//! Generic front-matter / markdown / YAML text-extraction helpers.
//!
//! Used to populate `AgentConfig` and `McpServerConfig` (see `agent_config`).
//! This is a distinct responsibility from the config domain model: these
//! functions know how to pull values and list items out of raw text lines,
//! not what an `AgentConfig` means.

use super::agent_config::{AgentConfig, McpServerConfig};

// Helper function to extract value from markdown field like "- **Name:** value"
pub(crate) fn extract_markdown_field_value(line: &str, field_prefix: &str) -> Option<String> {
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
pub(crate) fn extract_yaml_value(line: &str, key: &str) -> Option<String> {
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
pub(crate) fn parse_yaml_properties(
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

    // Check for entitlements (attribute FQNs requested for the agent's
    // delegated identity). An explicit key always clears the built-in
    // default list before it is (re)populated -- inline-array form
    // (`entitlements: ["...", "..."]`, matching the mcp_servers `args:`
    // convention), YAML-list form, or any other value -- so a narrower or
    // malformed request can never be silently widened back to
    // DEFAULT_AGENT_ENTITLEMENTS. An explicit empty list (`entitlements:`
    // with no following `- ` items) stays empty, distinguishable from an
    // absent key, which keeps the constructor-time default.
    if trimmed.starts_with("entitlements:") {
        agent.entitlements.clear();
        *in_entitlements_section = false;
        let inline = trimmed.strip_prefix("entitlements:").unwrap_or("").trim();
        if inline.starts_with('[') && inline.ends_with(']') {
            let inline_content = &inline[1..inline.len() - 1];
            agent.entitlements = inline_content
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if inline.is_empty() {
            *in_entitlements_section = true;
        }
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
