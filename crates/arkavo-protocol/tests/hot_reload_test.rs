#![allow(clippy::disallowed_methods)]

// Hot-reload configuration parser tests
// The async file watcher tests are excluded as they would run indefinitely
// Use the manual test scripts for integration testing of the hot-reload functionality

#[test]
fn test_agent_config_parser() {
    use arkavo_protocol::agent_config::parse_agents_config;

    let config = r#"
## Agent1

purpose: "Test agent one"
model: "model-1"
listen: "127.0.0.1:8080"
mdns: true

mcp_servers:
- name: filesystem
  command: "mcp-filesystem"
  args: ["/tmp", "--read-only"]
- name: git
  url: "http://localhost:3000/mcp"

ANTHROPIC_API_KEY: key1
OPENAI_API_KEY: key2

## Agent2

purpose: "Test agent two"
model: "model-2"
listen: "127.0.0.1:8081"
mdns: false
"#;

    let agents = parse_agents_config(config).expect("Failed to parse config");

    assert_eq!(agents.len(), 2);

    let agent1 = &agents[0];
    assert_eq!(agent1.name, "Agent1");
    assert_eq!(agent1.purpose, "Test agent one");
    assert_eq!(agent1.model, "model-1");
    assert_eq!(agent1.listen, "127.0.0.1:8080");
    assert!(agent1.mdns_enabled);
    assert_eq!(agent1.mcp_servers.len(), 2);
    assert_eq!(agent1.api_keys.len(), 2);

    let mcp1 = &agent1.mcp_servers[0];
    assert_eq!(mcp1.name, "filesystem");
    assert_eq!(mcp1.command, Some("mcp-filesystem".to_string()));
    assert_eq!(mcp1.args, vec!["/tmp", "--read-only"]);

    let mcp2 = &agent1.mcp_servers[1];
    assert_eq!(mcp2.name, "git");
    assert_eq!(mcp2.url, Some("http://localhost:3000/mcp".to_string()));

    let agent2 = &agents[1];
    assert_eq!(agent2.name, "Agent2");
    assert!(!agent2.mdns_enabled);
}

#[test]
fn test_empty_config_validation() {
    use arkavo_protocol::agent_config::parse_agents_config;

    let empty_config = "";
    let result = parse_agents_config(empty_config).expect("Should parse empty config");
    assert_eq!(result.len(), 0);

    let whitespace_config = "   \n  \t  \n  ";
    let result = parse_agents_config(whitespace_config).expect("Should parse whitespace");
    assert_eq!(result.len(), 0);
}

#[test]
fn test_malformed_config_handling() {
    use arkavo_protocol::agent_config::parse_agents_config;

    // Config with agent but missing required fields
    let config = r#"
## TestAgent

listen: "127.0.0.1:8080"
"#;

    let agents = parse_agents_config(config).expect("Should parse partial config");
    assert_eq!(agents.len(), 1);

    let agent = &agents[0];
    assert_eq!(agent.name, "TestAgent");
    assert_eq!(agent.purpose, ""); // Empty but not error
    assert_eq!(agent.model, ""); // Empty but not error
}
