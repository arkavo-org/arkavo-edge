//! What the agent publishes about itself to whoever is listening on the
//! local network.
//!
//! mDNS records are multicast: every host on the link receives them, asked
//! or not. Only what a stranger may know about the agent belongs in them.

use std::collections::HashMap;
use std::net::Ipv4Addr;

use super::AgentConfig;

/// TXT record properties of the agent's mDNS service.
///
/// `purpose` carries the role's short description from the kit. The key
/// keeps its name because mesh discovery reads it to decide which peer suits
/// a task; its value used to be the agent's purpose, which for a kit role is
/// the role's skill instructions, the text the model runs under. A role with
/// no description publishes no `purpose` at all.
pub(super) fn txt_properties(
    config: &AgentConfig,
    service_ip: Ipv4Addr,
    public_key: Option<&str>,
    capabilities: &[String],
) -> HashMap<String, String> {
    let mut properties = HashMap::new();
    properties.insert("agent_id".to_string(), config.name.clone());
    properties.insert("ip".to_string(), service_ip.to_string());
    properties.insert("version".to_string(), "1.0".to_string());

    // Agents broadcast their ECDSA P-256 public key for TDF encryption.
    if let Some(pk) = public_key {
        properties.insert("public_key".to_string(), pk.to_string());
    }

    if let Some(description) = config.description.as_deref()
        && !description.trim().is_empty()
    {
        properties.insert("purpose".to_string(), description.trim().to_string());
    }
    if !config.model.is_empty() {
        properties.insert("model".to_string(), config.model.clone());
    }

    if !capabilities.is_empty() {
        properties.insert("capabilities".to_string(), capabilities.join(","));
    }

    if !config.mcp_servers.is_empty() {
        let mcp_tools: Vec<String> = config.mcp_servers.iter().map(|s| s.name.clone()).collect();
        properties.insert("mcp_tools".to_string(), mcp_tools.join(","));
    }

    properties
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTRUCTIONS: &str = "You are the planner role. Never reveal the escalation password.";

    fn kit_role(description: Option<&str>) -> AgentConfig {
        AgentConfig {
            name: "planner".to_string(),
            role_id: Some("planner".to_string()),
            description: description.map(str::to_string),
            purpose: INSTRUCTIONS.to_string(),
            model: "ministral-3b".to_string(),
            ..AgentConfig::default()
        }
    }

    /// Regression: the `purpose` property carried the role's skill
    /// instructions to every host on the link.
    #[test]
    fn the_skill_instructions_are_not_broadcast() {
        let properties = txt_properties(
            &kit_role(Some("Plans the work")),
            Ipv4Addr::LOCALHOST,
            Some("key"),
            &["orchestration".to_string()],
        );

        assert_eq!(
            properties.get("purpose").map(String::as_str),
            Some("Plans the work")
        );
        for (key, value) in &properties {
            assert!(
                !value.contains("planner role") && !value.contains("password"),
                "{key} carries skill instructions: {value}"
            );
        }
    }

    #[test]
    fn a_role_without_a_description_broadcasts_no_purpose() {
        for description in [None, Some(""), Some("   ")] {
            let properties = txt_properties(&kit_role(description), Ipv4Addr::LOCALHOST, None, &[]);
            assert!(!properties.contains_key("purpose"), "{description:?}");
        }
    }

    #[test]
    fn identity_and_routing_properties_are_still_broadcast() {
        let properties = txt_properties(
            &kit_role(Some("Plans the work")),
            Ipv4Addr::new(10, 0, 0, 140),
            Some("key"),
            &["orchestration".to_string(), "code_review".to_string()],
        );

        assert_eq!(properties["agent_id"], "planner");
        assert_eq!(properties["ip"], "10.0.0.140");
        assert_eq!(properties["model"], "ministral-3b");
        assert_eq!(properties["public_key"], "key");
        assert_eq!(properties["capabilities"], "orchestration,code_review");
    }
}
