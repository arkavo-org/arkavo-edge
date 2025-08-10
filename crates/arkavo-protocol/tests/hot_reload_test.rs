use arkavo_protocol::{
    config::{BufferConfig, ServerConfig},
    server::A2aServer,
};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_hot_reload_metadata_update() {
    // Create a test AGENTS.md file
    let test_config = r#"
## TestAgent

purpose: "Initial test purpose"
model: "test-model"
listen: "127.0.0.1:9999"
mdns: false

ANTHROPIC_API_KEY: initial-key
"#;

    // Write initial configuration
    tokio::fs::write("AGENTS.md", test_config)
        .await
        .expect("Failed to write test config");

    // Create server
    let config = ServerConfig {
        bind_address: "127.0.0.1".to_string(),
        port: 9999,
        ..Default::default()
    };
    let server = A2aServer::new(config);

    // Set initial metadata
    server
        .set_agent_metadata(
            "TestAgent".to_string(),
            "Initial test purpose".to_string(),
            "test-model".to_string(),
        )
        .await;

    // Start file watcher
    server
        .start_file_watcher()
        .await
        .expect("Failed to start file watcher");

    // Give watcher time to initialize
    sleep(Duration::from_millis(500)).await;

    // Update configuration
    let updated_config = r#"
## TestAgent

purpose: "Updated test purpose"
model: "test-model"
listen: "127.0.0.1:9999"
mdns: false

ANTHROPIC_API_KEY: updated-key
MOONSHOT_API_KEY: new-key
"#;

    tokio::fs::write("AGENTS.md", updated_config)
        .await
        .expect("Failed to write updated config");

    // Wait for hot-reload to occur
    sleep(Duration::from_secs(2)).await;

    // Verify the update occurred (would need access to internal state)
    // In a real test, we'd check the agent metadata was updated

    // Clean up
    server.stop_file_watcher().await;
    let _ = tokio::fs::remove_file("AGENTS.md").await;
}

#[tokio::test]
async fn test_hot_reload_invalid_config_handling() {
    // Create initial valid config
    let test_config = r#"
## TestAgent

purpose: "Test purpose"
model: "test-model"
listen: "127.0.0.1:9998"
"#;

    tokio::fs::write("AGENTS.md", test_config)
        .await
        .expect("Failed to write test config");

    let config = ServerConfig {
        bind_address: "127.0.0.1".to_string(),
        port: 9998,
        ..Default::default()
    };
    let server = A2aServer::new(config);

    server
        .set_agent_metadata(
            "TestAgent".to_string(),
            "Test purpose".to_string(),
            "test-model".to_string(),
        )
        .await;

    server
        .start_file_watcher()
        .await
        .expect("Failed to start file watcher");

    sleep(Duration::from_millis(500)).await;

    // Write invalid configuration (different agent name)
    let invalid_config = r#"
## DifferentAgent

purpose: "Wrong agent"
model: "test-model"
listen: "127.0.0.1:9998"
"#;

    tokio::fs::write("AGENTS.md", invalid_config)
        .await
        .expect("Failed to write invalid config");

    // Wait for attempted reload
    sleep(Duration::from_secs(2)).await;

    // Server should still be operational despite invalid config
    // The error should be logged but agent continues with old config

    // Clean up
    server.stop_file_watcher().await;
    let _ = tokio::fs::remove_file("AGENTS.md").await;
}

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
    assert_eq!(agent1.mdns_enabled, true);
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
    assert_eq!(agent2.mdns_enabled, false);
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
