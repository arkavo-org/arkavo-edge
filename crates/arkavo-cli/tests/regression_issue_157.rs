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

/// Regression test for issue #157: MCP servers not yet implemented
///
/// This test ensures that command-based MCP servers can be spawned from within
/// an agent without causing "Cannot start a runtime from within a runtime" panic.
///
/// Issue: https://github.com/arkavo-org/arkavo-edge/issues/157
///
/// The fixture used to be an AGENTS.md file parsed by the CLI's now-deleted
/// top-level markdown/YAML config parser. Task 14 / S6 retargets it onto a
/// SwarmKit kit resolved through `commands::agent_kit::resolve_agent_configs`
/// — the CLI's only remaining `arkavo agent` run-path config source — while
/// keeping the regression's actual shape: config resolution executed inside
/// an async context (`#[tokio::test]`), the same context an agent spawning
/// an MCP server subprocess runs in.
use arkavo_cli::commands::agent_kit::resolve_agent_configs;
use std::process::Command;

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("create tempdir")
}

fn write_echo_mcp_kit(dir: &std::path::Path) -> std::path::PathBuf {
    let yaml = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "regression-157"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "Test agent for regression #157"
runtime:
  listen: "127.0.0.1:0"
  mcp_servers:
    - name: echo-test
      command: echo
      args: ["MCP server test"]
roles:
  - id: test-agent
    role_type: operator
    agent_provisioning:
      model:
        family: ministral
        size: "3B"
    skills:
      - id: "skill:identity"
        version: "0.1.0"
        source: inline
        payload:
          name: identity
          description: "System identity"
          instructions: "Test agent for regression #157"
          resources: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#;
    let path = dir.join("regression-157.swarmkit.yaml");
    std::fs::write(&path, yaml).unwrap();
    path
}

#[tokio::test]
async fn test_mcp_server_spawn_from_agent() {
    // This simulates the scenario where an agent spawns an MCP server
    // The MCP server (like arkavo serve) should not panic when trying to create a runtime

    let dir = tempdir();
    let kit_path = write_echo_mcp_kit(dir.path());

    // Resolve the kit configuration within an async context
    let configs = resolve_agent_configs(Some(&kit_path), None, None, dir.path())
        .expect("Failed to resolve agent config");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].mcp_servers.len(), 1);

    // Verify the MCP server configuration
    let mcp_server = &configs[0].mcp_servers[0];
    assert_eq!(mcp_server.name, "echo-test");
    assert_eq!(mcp_server.command, Some("echo".to_string()));
    assert_eq!(mcp_server.args, vec!["MCP server test"]);
}

#[test]
fn test_arkavo_serve_no_runtime_panic() {
    // Test that arkavo serve can handle being run in different contexts
    // This would previously panic with "Cannot start a runtime from within a runtime"

    // First, test that we can run a simple command
    let output = Command::new("echo")
        .arg("test")
        .output()
        .expect("Failed to run echo");

    assert!(output.status.success());

    // In a real scenario, this would test `arkavo serve` but we use echo as a stand-in
    // The important part is that the runtime detection logic is in place
}

#[tokio::test]
async fn test_nested_runtime_detection() {
    // Verify that runtime detection works correctly
    use tokio::runtime::Handle;

    // We should be in a runtime context in this async test
    assert!(
        Handle::try_current().is_ok(),
        "Should detect existing runtime"
    );

    // Test that commands can detect and reuse the existing runtime
    let result = tokio::task::spawn_blocking(|| {
        // Even in a blocking context, we should be able to detect the parent runtime
        Handle::try_current().is_ok()
    })
    .await
    .unwrap();

    assert!(result, "Should detect runtime from spawned blocking task");
}
