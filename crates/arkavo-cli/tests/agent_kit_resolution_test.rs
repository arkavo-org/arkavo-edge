//! Integration tests for the Task 5 `arkavo agent` run-path resolution:
//! `resolve_agent_configs` repoints config loading from AGENTS.md onto
//! SwarmKit kits (`-c/--config` > discovery > zero-config default).
//!
//! Exercises the pure `commands::agent_kit::resolve_agent_configs` function
//! directly (no process spawn, no server start) against temp directories.

use arkavo_cli::commands::agent_kit::{export_resolved_kit_path, resolve_agent_configs};
use arkavo_cli::commands::kit::init_kit;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

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

/// Review finding: `-n` with no kit anywhere must say no kit was found,
/// not list the hostname-derived zero-config default name as if it were a
/// selectable role id of some kit.
#[test]
fn name_flag_with_no_kit_anywhere_errors_with_no_kit_wording() {
    let dir = tempdir();

    let err = resolve_agent_configs(None, Some("bogus"), None, dir.path())
        .expect_err("-n with no kit anywhere must be a fatal error");

    let msg = err.to_string();
    assert!(
        msg.contains("no SwarmKit manifest found"),
        "message must say no kit exists: {msg}"
    );
    assert!(
        msg.contains("bogus"),
        "message should name the bad selector: {msg}"
    );
    assert!(
        msg.contains("arkavo kit init"),
        "message should point at kit creation: {msg}"
    );
    assert!(
        !msg.contains("available role ids"),
        "must not imply a kit with roles exists: {msg}"
    );
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

/// `ARKAVO_SWARMKIT_PATH` is process-global; this serializes the one test
/// in this binary that mutates it, matching the pattern used elsewhere in
/// the repo for this same env var (`arkavo-agui`'s
/// `swarm_flight_registry.rs`, via `serial_test`) without adding a new
/// dependency for a single call site.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Regression (finding 1): an explicit `-c` kit's preflight/KAS/budget
/// policy was silently not applied unless the kit also happened to be
/// cwd-discoverable, because server-side loaders (`arkavo_router::
/// load_agent_config`, spend plane, agui) re-discover their own kit from
/// process cwd/env and never see the CLI's resolved `-c` path.
///
/// `export_resolved_kit_path` is the fix's seam: after this call,
/// `ARKAVO_SWARMKIT_PATH` must point at the same kit `-c` resolved, and
/// `arkavo_swarmkit::discover_kit_path` from a *different* cwd (one that
/// does not itself contain the kit) must resolve to that same path —
/// proving a server-side loader booted from a different working directory
/// would now find it too.
#[test]
fn export_resolved_kit_path_makes_explicit_c_kit_discoverable_from_any_cwd() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("ARKAVO_SWARMKIT_PATH").ok();

    let kit_dir = tempdir();
    let report = init_kit(kit_dir.path(), "explicit-c-agent").expect("init_kit should succeed");
    let unrelated_cwd = tempdir(); // deliberately does not contain the kit

    export_resolved_kit_path(Some(&report.path), unrelated_cwd.path());

    let exported =
        std::env::var("ARKAVO_SWARMKIT_PATH").expect("export must set ARKAVO_SWARMKIT_PATH");
    let discovered = arkavo_swarmkit::discover_kit_path(unrelated_cwd.path());

    // Restore before any assert that might panic.
    // SAFETY: guarded by ENV_LOCK; no concurrent readers in this binary.
    unsafe {
        match &prev {
            Some(p) => std::env::set_var("ARKAVO_SWARMKIT_PATH", p),
            None => std::env::remove_var("ARKAVO_SWARMKIT_PATH"),
        }
    }

    assert_eq!(exported, report.path.to_string_lossy());
    assert_eq!(
        discovered.expect("discovery must succeed via the env var"),
        report.path,
        "a loader started from an unrelated cwd must still find the -c kit"
    );
}

/// No kit resolved anywhere (zero-config default) must not touch a
/// pre-existing `ARKAVO_SWARMKIT_PATH` — clearing it would be a surprising
/// side effect of a run that has no kit at all. Uses a stale (nonexistent)
/// path as the pre-existing value: that's the only way discovery can both
/// see an env var set *and* still resolve to "no kit" (a live env var
/// pointing at a real file is instead a successful resolution).
#[test]
fn export_resolved_kit_path_is_noop_when_no_kit_resolved() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("ARKAVO_SWARMKIT_PATH").ok();

    let stale_path = "/nonexistent/does-not-exist.swarmkit.yaml";
    // SAFETY: guarded by ENV_LOCK.
    unsafe {
        std::env::set_var("ARKAVO_SWARMKIT_PATH", stale_path);
    }

    let empty_dir = tempdir();
    export_resolved_kit_path(None, empty_dir.path());

    let after = std::env::var("ARKAVO_SWARMKIT_PATH").ok();

    // SAFETY: guarded by ENV_LOCK.
    unsafe {
        match &prev {
            Some(p) => std::env::set_var("ARKAVO_SWARMKIT_PATH", p),
            None => std::env::remove_var("ARKAVO_SWARMKIT_PATH"),
        }
    }

    assert_eq!(
        after.as_deref(),
        Some(stale_path),
        "no kit resolved must leave a pre-existing env var untouched, not cleared"
    );
}
