//! Integration tests for the Task 5 `arkavo agent` run-path resolution:
//! `resolve_agent_configs` repoints config loading from AGENTS.md onto
//! SwarmKit kits (`-c/--config` > discovery > zero-config default).
//!
//! Exercises the pure `commands::agent_kit::resolve_agent_configs` function
//! directly (no process spawn, no server start) against temp directories.

use arkavo_cli::commands::agent_kit::resolve_agent_configs;
use arkavo_cli::commands::kit::init_kit;
use std::fs;
use std::path::Path;

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("create tempdir")
}

/// A two-role kit written directly as YAML (`init_kit` only produces
/// single-role kits), with a kit-level `runtime` block and per-role models.
fn write_multi_role_kit(dir: &Path, file_name: &str) {
    let yaml = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "multi-role-demo"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "demonstrate multi-role resolution"
runtime:
  mode: specialist
  listen: "127.0.0.1:8342"
  mdns: false
  mcp_servers:
    - name: filesystem
      command: arkavo
      args: ["mcp", "filesystem"]
roles:
  - id: planner
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
          instructions: "You are the planner role."
          resources: []
    mcp_tools: []
    handoffs: []
  - id: worker
    role_type: operator
    agent_provisioning:
      model:
        family: gemma
        size: "E2B"
    skills:
      - id: "skill:identity"
        version: "0.1.0"
        source: inline
        payload:
          name: identity
          description: "System identity"
          instructions: "You are the worker role."
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
    fs::write(dir.join(file_name), yaml).unwrap();
}

#[test]
fn explicit_config_path_resolves_one_agent_config() {
    let dir = tempdir();
    let report = init_kit(dir.path(), "solo-agent").expect("init_kit should succeed");

    let configs = resolve_agent_configs(Some(&report.path), None, None, dir.path())
        .expect("resolution should succeed");

    assert_eq!(configs.len(), 1);
    assert_eq!(
        configs[0].name, "agent",
        "role id from init_kit's single role"
    );
    assert_eq!(configs[0].model, "ministral-3b");
    assert!(configs[0].mdns_enabled);
    assert_eq!(configs[0].listen, "0.0.0.0:0");
}

#[test]
fn discovery_finds_kit_in_dot_arkavo_without_explicit_c() {
    let dir = tempdir();
    init_kit(dir.path(), "discoverable-agent").expect("init_kit should succeed");

    let configs =
        resolve_agent_configs(None, None, None, dir.path()).expect("discovery should find the kit");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].name, "agent");
}

#[test]
fn multi_role_kit_resolves_same_count_and_order_as_roles_declared() {
    let dir = tempdir();
    write_multi_role_kit(dir.path(), "multi.swarmkit.yaml");
    let path = dir.path().join("multi.swarmkit.yaml");

    let configs =
        resolve_agent_configs(Some(&path), None, None, dir.path()).expect("resolution succeeds");

    assert_eq!(
        configs.len(),
        2,
        "one AgentConfig per role, same as today's multi-section AGENTS.md output"
    );
    assert_eq!(configs[0].name, "planner", "manifest role order preserved");
    assert_eq!(configs[1].name, "worker");
    assert_eq!(configs[0].model, "ministral-3b");
    assert_eq!(configs[1].model, "gemma-4-e2b");
    // Kit-level runtime fields apply uniformly to every role.
    for cfg in &configs {
        assert_eq!(cfg.listen, "127.0.0.1:8342");
        assert!(!cfg.mdns_enabled);
        assert_eq!(
            cfg.mode,
            arkavo_protocol::agent_config::AgentMode::Specialist
        );
        assert!(
            cfg.api_keys.is_empty(),
            "api_keys are env-only, never from the kit"
        );
        assert_eq!(cfg.mcp_servers.len(), 1);
        assert_eq!(cfg.mcp_servers[0].name, "filesystem");
        assert_eq!(cfg.mcp_servers[0].command.as_deref(), Some("arkavo"));
        assert_eq!(
            cfg.mcp_servers[0].args,
            vec!["mcp".to_string(), "filesystem".to_string()]
        );
    }
}

#[test]
fn name_flag_selects_exactly_one_role() {
    let dir = tempdir();
    write_multi_role_kit(dir.path(), "multi.swarmkit.yaml");
    let path = dir.path().join("multi.swarmkit.yaml");

    let configs = resolve_agent_configs(Some(&path), Some("worker"), None, dir.path())
        .expect("resolution succeeds");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].name, "worker");
    assert_eq!(configs[0].purpose, "You are the worker role.");
}

#[test]
fn name_flag_with_no_matching_role_errors_and_lists_role_ids() {
    let dir = tempdir();
    write_multi_role_kit(dir.path(), "multi.swarmkit.yaml");
    let path = dir.path().join("multi.swarmkit.yaml");

    let err = resolve_agent_configs(Some(&path), Some("bogus"), None, dir.path())
        .expect_err("unknown role name must be a fatal error");

    let msg = err.to_string();
    assert!(
        msg.contains("bogus"),
        "message should name the bad selector: {msg}"
    );
    assert!(
        msg.contains("planner"),
        "message should list available role ids: {msg}"
    );
    assert!(
        msg.contains("worker"),
        "message should list available role ids: {msg}"
    );
}

#[test]
fn port_override_replaces_only_the_port_part_of_listen() {
    let dir = tempdir();
    let report = init_kit(dir.path(), "port-agent").expect("init_kit should succeed");

    let configs = resolve_agent_configs(Some(&report.path), None, Some(9999), dir.path())
        .expect("resolution should succeed");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].listen, "0.0.0.0:9999");
}

#[test]
fn agents_md_present_without_a_kit_falls_back_to_default_and_does_not_parse_it() {
    let dir = tempdir();
    fs::write(
        dir.path().join("AGENTS.md"),
        "# AGENTS.md — should-never-appear\npurpose: this must not be parsed\n",
    )
    .unwrap();

    let configs = resolve_agent_configs(None, None, None, dir.path())
        .expect("must fall through to the zero-config default, not error");

    assert_eq!(configs.len(), 1);
    assert_ne!(
        configs[0].name, "should-never-appear",
        "AGENTS.md content must never be parsed by the kit run path"
    );
    assert_eq!(configs[0].purpose, "A general-purpose AI agent");
}

#[test]
fn nothing_present_falls_back_to_default_config() {
    let dir = tempdir();

    let configs = resolve_agent_configs(None, None, None, dir.path()).expect("zero-config default");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].purpose, "A general-purpose AI agent");
    assert_eq!(configs[0].listen, "0.0.0.0:0");
    assert!(configs[0].mdns_enabled);
}

#[test]
fn explicit_config_path_to_invalid_yaml_is_fatal_with_no_default_fallback() {
    let dir = tempdir();
    let path = dir.path().join("broken.swarmkit.yaml");
    fs::write(&path, "not: [valid, yaml: structure").unwrap();

    let err = resolve_agent_configs(Some(&path), None, None, dir.path())
        .expect_err("invalid YAML at an explicit -c path must never fall back silently");
    assert!(!err.to_string().is_empty());
}
