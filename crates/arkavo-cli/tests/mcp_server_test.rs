#![allow(clippy::disallowed_methods)]
#![allow(clippy::future_not_send)]
#![allow(dead_code)]
#![allow(clippy::format_push_string)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unnecessary_debug_formatting)]
#![allow(clippy::lines_filter_map_ok)]
#![allow(clippy::manual_strip)]
#![allow(clippy::needless_continue)]
#![allow(unused_imports)]
#![allow(clippy::zombie_processes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(unreachable_pub)]

#[test]
fn test_mcp_server_config_parsing() {
    let config_content = r#"# AGENTS.md

## my-agent
purpose: Test agent
model:   test
listen:  localhost:8080
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--allow-write"]
  - name: git
    command: mcp-git
    args: []
  - name: external
    url: http://localhost:3000
"#;

    let result = arkavo_cli::commands::agent::parse_agents_config(config_content);
    assert!(result.is_ok());

    let configs = result.unwrap();
    assert_eq!(configs.len(), 1);

    let config = &configs[0];
    assert_eq!(config.name, "my-agent");
    assert_eq!(config.mcp_servers.len(), 3);

    // Check command-based server
    assert_eq!(config.mcp_servers[0].name, "filesystem");
    assert_eq!(
        config.mcp_servers[0].command,
        Some("mcp-filesystem".to_string())
    );
    assert_eq!(config.mcp_servers[0].args, vec!["--allow-write"]);
    assert!(config.mcp_servers[0].url.is_none());

    // Check another command-based server
    assert_eq!(config.mcp_servers[1].name, "git");
    assert_eq!(config.mcp_servers[1].command, Some("mcp-git".to_string()));
    assert!(config.mcp_servers[1].args.is_empty());

    // Check URL-based server
    assert_eq!(config.mcp_servers[2].name, "external");
    assert!(config.mcp_servers[2].command.is_none());
    assert_eq!(
        config.mcp_servers[2].url,
        Some("http://localhost:3000".to_string())
    );
}
