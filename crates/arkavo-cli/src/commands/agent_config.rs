//! AGENTS.md parsing.
//!
//! Two shapes exist in the wild: a YAML frontmatter block fenced by `---`, and markdown
//! sections (`## agent-name` followed by `key: value` lines, the shape every file under
//! `examples/` uses). A file may legitimately carry both — frontmatter for cross-cutting
//! blocks such as `budget:` or `kas:`, a `##` section for the agent's identity — so both
//! halves are parsed and merged. Parsing only the frontmatter used to discard the entire
//! body, which dropped the agent's name, purpose, model and listen address in silence and
//! left the agent running with an empty identity.
//!
//! Precedence: a `##` section overrides the frontmatter for every field it sets to a
//! non-default value, and inherits the rest from it. Setting a field to its own default
//! (`mdns: true`, `mode: orchestrator`) is indistinguishable from not setting it, so it
//! does not override the frontmatter.

use super::agent_config_markdown::parse_markdown_sections;
use super::agent_config_yaml::parse_yaml_properties;

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            purpose: String::new(),
            model: String::new(),
            mode: arkavo_protocol::agent_config::AgentMode::default(),
            listen: "0.0.0.0:0".to_string(), // Dynamic port
            mdns_enabled: true,              // Zero-config discovery
            mcp_servers: Vec::new(),
            api_keys: std::collections::HashMap::new(),
            quiet: true,
            peers: Vec::new(),
            a2a_enabled: true,
            a2a_service_type: None,
            swarm: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}

/// Parse an AGENTS.md file into one config per declared agent.
///
/// # Errors
/// Returns an error when a declared agent has no name: mDNS registration and the A2A
/// agent card both require one, so an anonymous agent must not reach startup.
pub fn parse_agents_config(content: &str) -> Result<Vec<AgentConfig>, Box<dyn std::error::Error>> {
    let agents = match split_frontmatter(content) {
        Some((frontmatter, body)) => {
            let base = parse_frontmatter(frontmatter);
            let sections: Vec<AgentConfig> = parse_markdown_sections(body)
                .into_iter()
                .filter(declares_configuration)
                .collect();
            if sections.is_empty() {
                vec![base]
            } else {
                sections.into_iter().map(|s| merge_over(&base, s)).collect()
            }
        }
        None => parse_markdown_sections(content),
    };

    if agents.iter().any(|a| a.name.trim().is_empty()) {
        return Err(
            "AGENTS.md declares an agent with no name: add a `name:` key to the \
                    YAML frontmatter, or a `## <agent-name>` section heading. An agent \
                    without a name cannot register over mDNS or publish an A2A agent card."
                .into(),
        );
    }

    Ok(agents)
}

/// Split `---`-fenced frontmatter from the markdown body that follows it.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let after_open = content.strip_prefix("---")?;
    let end_idx = after_open.find("\n---")?;
    Some((
        &after_open[..end_idx],
        &after_open[end_idx + "\n---".len()..],
    ))
}

fn parse_frontmatter(frontmatter: &str) -> AgentConfig {
    let mut agent = AgentConfig::default();
    let mut in_mcp_section = false;
    let mut current_mcp_server: Option<McpServerConfig> = None;
    let mut in_peers_section = false;
    let mut in_args_section = false;
    let mut in_a2a_section = false;
    let mut in_purpose_multiline = false;
    let mut purpose_lines: Vec<String> = Vec::new();

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
    agent
}

/// Does this markdown section actually declare configuration?
///
/// Documentation headings (`## Usage`, `## How It Works`) and the H1 title parse into a
/// config that is a bare default apart from its heading text; only a section that set a
/// field carries configuration worth merging. Comparing against the default rather than
/// testing fields individually keeps this correct as fields are added.
fn declares_configuration(section: &AgentConfig) -> bool {
    *section
        != AgentConfig {
            name: section.name.clone(),
            ..AgentConfig::default()
        }
}

/// Overlay a `##` section onto the frontmatter defaults.
fn merge_over(base: &AgentConfig, section: AgentConfig) -> AgentConfig {
    let default = AgentConfig::default();
    let mut merged = base.clone();
    if section.name != default.name {
        merged.name = section.name;
    }
    if section.purpose != default.purpose {
        merged.purpose = section.purpose;
    }
    if section.model != default.model {
        merged.model = section.model;
    }
    if section.mode != default.mode {
        merged.mode = section.mode;
    }
    if section.listen != default.listen {
        merged.listen = section.listen;
    }
    if section.mdns_enabled != default.mdns_enabled {
        merged.mdns_enabled = section.mdns_enabled;
    }
    if section.quiet != default.quiet {
        merged.quiet = section.quiet;
    }
    if section.a2a_enabled != default.a2a_enabled {
        merged.a2a_enabled = section.a2a_enabled;
    }
    if section.a2a_service_type.is_some() {
        merged.a2a_service_type = section.a2a_service_type;
    }
    if section.swarm.is_some() {
        merged.swarm = section.swarm;
    }
    if !section.peers.is_empty() {
        merged.peers = section.peers;
    }
    if !section.mcp_servers.is_empty() {
        merged.mcp_servers = section.mcp_servers;
    }
    // API keys are additive: the frontmatter usually holds the shared credentials while a
    // section may add or replace individual ones.
    merged.api_keys.extend(section.api_keys);
    merged
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

        // The runtime loads KAS config from AGENTS.md via arkavo-router, not
        // through the CLI AgentConfig above. Verify the trusted_roots block in
        // this fixture is no longer parsed-then-discarded on that path.
        let mut file = tempfile::NamedTempFile::with_suffix(".md").unwrap();
        use std::io::Write as _;
        write!(file, "{content}").unwrap();
        let runtime_config =
            arkavo_router::preflight::load_agent_config_from_agents_md(file.path()).unwrap();
        let kas = runtime_config.kas.expect("kas config should parse");
        assert!(kas.enabled);
        assert_eq!(kas.key_id.as_deref(), Some("bridge-demo-key-1"));
        assert_eq!(kas.trusted_roots.len(), 1);
        assert_eq!(
            kas.trusted_roots[0].did,
            "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
        );
        assert_eq!(
            kas.trusted_roots[0].name.as_deref(),
            Some("Demo Root Authority")
        );
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

    // Regression: frontmatter used to short-circuit the parse, so a file carrying both a
    // `budget:` block and an examples-style `## agent-name` section lost its whole
    // identity — the agent started with an empty name, no model hint and a random port.
    #[test]
    fn frontmatter_does_not_discard_markdown_agent_section() {
        let content = r#"---
budget:
  cloud_policy: cloud_within_cap
  max_cost_per_session: 0.20
---
# AGENTS.md

## self-improvement-orchestrator
purpose: Orchestrate codebase self-improvement by coordinating specialized agents
model:   gpt-6-astra
listen:  0.0.0.0:8400

discovery:
  mdns: true
"#;
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "self-improvement-orchestrator");
        assert_eq!(agents[0].model, "gpt-6-astra");
        assert_eq!(agents[0].listen, "0.0.0.0:8400");
        assert!(agents[0].purpose.starts_with("Orchestrate codebase"));
    }

    // The frontmatter supplies whatever the section leaves out, including values that are
    // impossible to spell as an override because they equal the field default.
    #[test]
    fn markdown_section_inherits_frontmatter_defaults() {
        let content = "---\nname: ignored-by-section\nmodel: ministral-3b\nmode: specialist\nmdns: false\nswarm: research\n---\n\n## real-agent\npurpose: \"Does the work\"\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "real-agent");
        assert_eq!(agents[0].purpose, "Does the work");
        assert_eq!(agents[0].model, "ministral-3b");
        assert_eq!(
            agents[0].mode,
            arkavo_protocol::agent_config::AgentMode::Specialist
        );
        assert!(!agents[0].mdns_enabled);
        assert_eq!(agents[0].swarm.as_deref(), Some("research"));
    }

    #[test]
    fn markdown_sections_override_frontmatter_per_field() {
        let content = "---\nname: base\nmodel: ministral-3b\nlisten: 0.0.0.0:9000\n---\n\n## first\nmodel: gpt-6-astra\n\n## second\nlisten: 0.0.0.0:9100\n";
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].name, "first");
        assert_eq!(agents[0].model, "gpt-6-astra");
        assert_eq!(agents[0].listen, "0.0.0.0:9000"); // inherited
        assert_eq!(agents[1].name, "second");
        assert_eq!(agents[1].model, "ministral-3b"); // inherited
        assert_eq!(agents[1].listen, "0.0.0.0:9100");
    }

    // Documentation headings and fenced examples are prose, not agents: every published
    // example with frontmatter has a body full of them.
    #[test]
    fn frontmatter_documentation_body_declares_no_agents() {
        let content = r#"---
name: kas-agent
purpose: "Demonstrates KAS as an A2A capability"
model: ministral-3b
---

# KAS Agent

## Capabilities

- **kas.publicKey** - Retrieve the KAS public key

## How It Works

1. Delegation tokens are verified
2. ABAC policy is evaluated

## Usage

```bash
# Get public key
arkavo agent run
```

## Agent Card

```json
{
  "skills": [
    { "id": "kas.rewrap", "name": "TDF Key Rewrap" }
  ]
}
```
"#;
        let agents = parse_agents_config(content).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "kas-agent");
        assert_eq!(agents[0].model, "ministral-3b");
    }

    // An anonymous agent cannot register over mDNS or publish an agent card, so the
    // parser rejects the file instead of letting startup proceed with an empty identity.
    #[test]
    fn nameless_config_is_rejected() {
        let content =
            "---\nbudget:\n  cloud_policy: cloud_within_cap\n---\n\n# Just documentation\n";
        let err = parse_agents_config(content).unwrap_err().to_string();
        assert!(err.contains("no name"), "unexpected error: {err}");
    }
}
