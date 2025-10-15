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

#[cfg(test)]
mod mcp_agent_tests {
    use arkavo_cli::commands::agent::parse_agents_config;

    #[test]
    fn test_parse_agents_with_mcp_servers() {
        let agents_md = r#"# AGENTS.md

## test-agent
purpose: Test agent with MCP servers
model:   ollama://127.0.0.1:11434/qwen3:0.6b
listen:  0.0.0.0:8342
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--allow-write", "--root", "/tmp"]
  - name: git
    command: mcp-git
    args: []
  - name: external
    url: http://localhost:8080

## another-agent
purpose: Another agent without MCP
model:   openai://gpt-4
listen:  0.0.0.0:8343
"#;

        let agents = parse_agents_config(agents_md).unwrap();
        assert_eq!(agents.len(), 2);

        // Check first agent with MCP servers
        let agent1 = &agents[0];
        assert_eq!(agent1.name, "test-agent");
        assert_eq!(agent1.purpose, "Test agent with MCP servers");
        assert_eq!(agent1.model, "ollama://127.0.0.1:11434/qwen3:0.6b");
        assert_eq!(agent1.listen, "0.0.0.0:8342");
        assert_eq!(agent1.mcp_servers.len(), 3);

        // Check filesystem server
        let fs_server = &agent1.mcp_servers[0];
        assert_eq!(fs_server.name, "filesystem");
        assert_eq!(fs_server.command, Some("mcp-filesystem".to_string()));
        assert_eq!(fs_server.args, vec!["--allow-write", "--root", "/tmp"]);
        assert_eq!(fs_server.url, None);

        // Check git server
        let git_server = &agent1.mcp_servers[1];
        assert_eq!(git_server.name, "git");
        assert_eq!(git_server.command, Some("mcp-git".to_string()));
        assert_eq!(git_server.args, Vec::<String>::new());
        assert_eq!(git_server.url, None);

        // Check external server
        let ext_server = &agent1.mcp_servers[2];
        assert_eq!(ext_server.name, "external");
        assert_eq!(ext_server.command, None);
        assert_eq!(ext_server.args, Vec::<String>::new());
        assert_eq!(ext_server.url, Some("http://localhost:8080".to_string()));

        // Check second agent without MCP servers
        let agent2 = &agents[1];
        assert_eq!(agent2.name, "another-agent");
        assert_eq!(agent2.mcp_servers.len(), 0);
    }

    #[test]
    fn test_parse_mcp_server_config_variations() {
        let agents_md = r#"# AGENTS.md

## test-agent
purpose: Test different MCP configurations
model:   ollama://127.0.0.1:11434/test
listen:  0.0.0.0:8342
mcp_servers:
  - name: single-arg
    command: test-tool
    args: ["--flag"]
  - name: no-args
    command: test-tool
    args: []
  - name: url-only
    url: https://example.com/mcp
"#;

        let agents = parse_agents_config(agents_md).unwrap();
        assert_eq!(agents.len(), 1);

        let agent = &agents[0];
        assert_eq!(agent.mcp_servers.len(), 3);

        // Test single arg
        assert_eq!(agent.mcp_servers[0].args, vec!["--flag"]);

        // Test no args
        assert_eq!(agent.mcp_servers[1].args.len(), 0);

        // Test URL only
        assert_eq!(
            agent.mcp_servers[2].url,
            Some("https://example.com/mcp".to_string())
        );
    }

    #[test]
    fn test_agent_without_mcp_servers() {
        let agents_md = r#"# AGENTS.md

## minimal-agent
purpose: Minimal agent
model:   ollama://127.0.0.1:11434/test
listen:  0.0.0.0:8342
"#;

        let agents = parse_agents_config(agents_md).unwrap();
        assert_eq!(agents.len(), 1);

        let agent = &agents[0];
        assert_eq!(agent.name, "minimal-agent");
        assert_eq!(agent.mcp_servers.len(), 0);
    }
}
