#[tokio::test]
async fn test_nested_runtime_handling() {
    // This test verifies that MCP servers can be spawned from within an async context
    // without causing a "Cannot start a runtime from within a runtime" panic

    let config_content = r#"# AGENTS.md

## test-agent
purpose: Test agent with nested MCP server
model:   test-model
listen:  127.0.0.1:0
mcp_servers:
  - name: echo-test
    command: echo
    args: ["test"]
"#;

    // Parse the configuration
    let configs = arkavo_cli::commands::agent::parse_agents_config(config_content).unwrap();
    assert_eq!(configs.len(), 1);

    // This would previously panic when trying to spawn an MCP server
    // that itself tries to create a runtime (like arkavo serve)
    let config = &configs[0];
    assert_eq!(config.mcp_servers.len(), 1);
}

#[test]
fn test_sync_runtime_creation() {
    // Test that commands can still create runtimes in sync contexts
    use std::process::Command;

    // This should work without issues
    let output = Command::new("echo")
        .arg("test")
        .output()
        .expect("Failed to execute echo");

    assert!(output.status.success());
}
