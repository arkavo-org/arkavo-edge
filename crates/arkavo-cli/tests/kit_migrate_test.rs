//! Integration tests for `arkavo kit migrate-from-agents-md`.
//!
//! Exercises the pure `commands::kit::migrate_from_agents_md` function
//! directly against temp files, plus `commands::kit::execute` for the
//! CLI-level argument/exit-code contract, per the Task 2 brief.

use arkavo_cli::commands::kit;
use arkavo_cli::commands::kit::migrate_from_agents_md;
use std::fs;

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("create tempdir")
}

#[test]
fn single_agent_new_format_migrates_to_valid_kit() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md — demo-migrated

## Agent Identity

- **Name:** demo-migrated
- **Mission:** Handles customer support triage and routes tickets to specialists

## Runtime Configuration

```yaml
listen: 0.0.0.0:8342
mdns: true
```
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path).expect("migration should succeed");
    assert!(
        report.unmapped.is_empty(),
        "unexpected unmapped: {:?}",
        report.unmapped
    );

    let content = fs::read_to_string(&report.path).unwrap();
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("migrated kit must validate");
    let recomputed = arkavo_swarmkit::kit_id_for(&manifest).unwrap();
    assert_eq!(manifest.kit.id, recomputed);
    assert_eq!(manifest.kit.id, report.kit_id);

    assert_eq!(manifest.roles.len(), 1);
    assert_eq!(manifest.roles[0].id, "demo-migrated");
    assert_eq!(
        manifest.objective.goal,
        "Handles customer support triage and routes tickets to specialists"
    );
    let skill = &manifest.roles[0].skills[0];
    let instructions = skill.payload.as_ref().unwrap()["instructions"]
        .as_str()
        .unwrap();
    assert_eq!(
        instructions,
        "Handles customer support triage and routes tickets to specialists"
    );

    let runtime = manifest.runtime.expect("runtime block required");
    assert_eq!(runtime.local_dev, Some(true));
    assert_eq!(runtime.listen.as_deref(), Some("0.0.0.0:8342"));
    assert_eq!(runtime.mdns, Some(true));
}

#[test]
fn old_format_maps_purpose_model_listen_mdns_into_runtime() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md

## worker-old

purpose: Processes queued jobs and reports status
model: ministral-3b
listen: 0.0.0.0:9100
mdns: false
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path).expect("migration should succeed");
    assert!(
        report.unmapped.is_empty(),
        "unexpected unmapped: {:?}",
        report.unmapped
    );

    let content = fs::read_to_string(&report.path).unwrap();
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("migrated kit must validate");

    assert_eq!(manifest.roles[0].id, "worker-old");
    let model = manifest.roles[0]
        .agent_provisioning
        .model
        .as_ref()
        .expect("model must be set");
    assert_eq!(model.family, "ministral");
    assert_eq!(model.size.as_deref(), Some("3B"));

    let runtime = manifest.runtime.expect("runtime block required");
    assert_eq!(runtime.listen.as_deref(), Some("0.0.0.0:9100"));
    assert_eq!(runtime.mdns, Some(false));
}

#[test]
fn multi_agent_sections_produce_one_kit_with_two_roles() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md

## agent-one

purpose: Handles inbound requests

## agent-two

purpose: Handles outbound notifications
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path).expect("migration should succeed");
    assert!(
        report.unmapped.is_empty(),
        "unexpected unmapped: {:?}",
        report.unmapped
    );

    let content = fs::read_to_string(&report.path).unwrap();
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("migrated kit must validate");

    assert_eq!(manifest.roles.len(), 2);
    assert_eq!(manifest.roles[0].id, "agent-one");
    assert_eq!(manifest.roles[1].id, "agent-two");
    assert_eq!(manifest.objective.goal, "Handles inbound requests");

    let second_instructions = manifest.roles[1].skills[0].payload.as_ref().unwrap()["instructions"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(second_instructions, "Handles outbound notifications");
}

#[test]
fn mcp_servers_block_populates_runtime_and_role_grants() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md

## tool-agent

purpose: Uses filesystem and git tools
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: ["--read-only"]
  - name: git
    command: mcp-git
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path).expect("migration should succeed");
    assert!(
        report.unmapped.is_empty(),
        "unexpected unmapped: {:?}",
        report.unmapped
    );

    let content = fs::read_to_string(&report.path).unwrap();
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("migrated kit must validate");

    let runtime = manifest.runtime.expect("runtime block required");
    assert_eq!(runtime.mcp_servers.len(), 2);
    assert!(runtime.mcp_servers.iter().any(|s| s.name == "filesystem"
        && s.command.as_deref() == Some("mcp-filesystem")
        && s.args == vec!["--read-only".to_string()]));
    assert!(
        runtime
            .mcp_servers
            .iter()
            .any(|s| s.name == "git" && s.command.as_deref() == Some("mcp-git"))
    );

    let grants = &manifest.roles[0].mcp_tools;
    assert_eq!(grants.len(), 2);
    assert!(grants.iter().any(|g| g.server == "filesystem"));
    assert!(grants.iter().any(|g| g.server == "git"));
}

#[test]
fn api_keys_are_never_copied_and_reported_unmapped() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        "# AGENTS.md\n\n## secret-agent\n\npurpose: Talks to Moonshot\nMOONSHOT_API_KEY: sk-super-secret-value\n",
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path)
        .expect("kit should still be written even with unmapped fields");
    assert!(
        report.unmapped.iter().any(|l| l.contains("api_keys")),
        "api_keys must be listed as unmapped: {:?}",
        report.unmapped
    );

    let written = fs::read_to_string(&out_path).unwrap();
    assert!(
        !written.contains("sk-super-secret-value"),
        "kit YAML must never contain an API key value"
    );
    assert!(
        !written.contains("MOONSHOT_API_KEY"),
        "kit YAML must never contain an API key name"
    );

    // The CLI-level command must exit non-zero when fields are unmapped.
    let out_path_2 = dir.path().join("agent2.swarmkit.yaml");
    let args = vec![
        "migrate-from-agents-md".to_string(),
        "--in".to_string(),
        in_path.to_string_lossy().to_string(),
        "--out".to_string(),
        out_path_2.to_string_lossy().to_string(),
    ];
    let result = kit::execute(&args);
    assert!(
        result.is_err(),
        "command must return Err (non-zero exit) when fields are unmapped"
    );
}

#[test]
fn missing_in_or_out_flags_is_a_usage_error() {
    let no_flags = kit::execute(&["migrate-from-agents-md".to_string()]);
    assert!(no_flags.is_err());

    let only_in = kit::execute(&[
        "migrate-from-agents-md".to_string(),
        "--in".to_string(),
        "AGENTS.md".to_string(),
    ]);
    assert!(only_in.is_err());

    let only_out = kit::execute(&[
        "migrate-from-agents-md".to_string(),
        "--out".to_string(),
        "agent.swarmkit.yaml".to_string(),
    ]);
    assert!(only_out.is_err());
}

#[test]
fn missing_out_parent_dir_errors_without_creating_it() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        "# AGENTS.md\n\n## hello-agent\n\npurpose: Says hello\n",
    )
    .unwrap();
    let out_path = dir
        .path()
        .join("missing-parent")
        .join("agent.swarmkit.yaml");

    let result = migrate_from_agents_md(&in_path, &out_path);
    assert!(
        result.is_err(),
        "must error when --out's parent directory does not exist"
    );
    assert!(
        !out_path.exists(),
        "must not create the missing parent directory"
    );
}

#[test]
fn legacy_only_fields_are_reported_unmapped_without_failing_hard() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        "---\nname: full-agent\npurpose: \"Exercises every unmapped field\"\nmodel: ministral-3b\nswarm: research-domain\nquiet: false\na2a:\n  enabled: false\n  service_type: \"custom\"\npeers:\n  - \"localhost:9001\"\n---\n",
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path).expect("kit should still be written");
    for expected in ["swarm", "peers", "a2a_enabled", "a2a_service_type", "quiet"] {
        assert!(
            report.unmapped.iter().any(|l| l.contains(expected)),
            "expected {expected:?} to be reported unmapped, got {:?}",
            report.unmapped
        );
    }
}

#[test]
fn existing_out_path_errors_without_modifying_it() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        "# AGENTS.md\n\n## hello-agent\n\npurpose: Says hello\n",
    )
    .unwrap();
    let out_path = dir.path().join("existing.swarmkit.yaml");
    fs::write(&out_path, "not a kit").unwrap();

    let result = migrate_from_agents_md(&in_path, &out_path);
    assert!(result.is_err(), "must refuse to overwrite existing --out");

    let content = fs::read_to_string(&out_path).unwrap();
    assert_eq!(
        content, "not a kit",
        "existing --out file must be unchanged"
    );
}

/// Regression for finding 1: a kit has exactly one `runtime` block, built
/// from the first AGENTS.md agent section. A second section that declares a
/// different `listen`/`mode`/`mdns` must not vanish silently — it has to
/// show up as `unmapped:` lines so `kit::execute` exits non-zero.
#[test]
fn divergent_runtime_settings_across_agents_are_reported_unmapped() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md

## primary

purpose: Handles inbound requests
listen: 0.0.0.0:8342
mode: orchestrator
mdns: true

## worker

purpose: Handles background jobs
listen: 0.0.0.0:9100
mode: specialist
mdns: false
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path)
        .expect("kit should still be written despite the divergence");
    assert!(out_path.exists(), "kit file must still be written");

    for expected in [
        "unmapped: listen (agent 'worker' differs from kit-level runtime.listen)",
        "unmapped: mode (agent 'worker' differs from kit-level runtime.mode)",
        "unmapped: mdns (agent 'worker' differs from kit-level runtime.mdns)",
    ] {
        assert!(
            report.unmapped.iter().any(|l| l == expected),
            "expected {expected:?} in unmapped, got {:?}",
            report.unmapped
        );
    }

    // The CLI-level command must exit non-zero when a divergence exists.
    let out_path_2 = dir.path().join("agent2.swarmkit.yaml");
    let args = vec![
        "migrate-from-agents-md".to_string(),
        "--in".to_string(),
        in_path.to_string_lossy().to_string(),
        "--out".to_string(),
        out_path_2.to_string_lossy().to_string(),
    ];
    let result = kit::execute(&args);
    assert!(
        result.is_err(),
        "divergent runtime settings must cause a non-zero exit"
    );
}

/// Companion to the above: identical `listen`/`mode`/`mdns` across every
/// section must not be reported unmapped, and the command must exit 0.
#[test]
fn identical_runtime_settings_across_agents_exit_zero() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md

## primary

purpose: Handles inbound requests
listen: 0.0.0.0:8342
mode: orchestrator
mdns: true

## worker

purpose: Handles background jobs
listen: 0.0.0.0:8342
mode: orchestrator
mdns: true
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path).expect("migration should succeed");
    assert!(
        report.unmapped.is_empty(),
        "identical runtime settings must not be reported unmapped: {:?}",
        report.unmapped
    );

    let args = vec![
        "migrate-from-agents-md".to_string(),
        "--in".to_string(),
        in_path.to_string_lossy().to_string(),
        "--out".to_string(),
        dir.path()
            .join("agent2.swarmkit.yaml")
            .to_string_lossy()
            .to_string(),
    ];
    assert!(
        kit::execute(&args).is_ok(),
        "identical runtime settings must exit zero"
    );
}

/// Regression for finding 2: two agent sections that both define an MCP
/// server named `foo` but with different commands must not silently keep
/// the first definition — the conflict has to surface as `unmapped:`.
#[test]
fn conflicting_same_name_mcp_servers_are_reported_unmapped() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md

## primary

purpose: Handles inbound requests
mcp_servers:
  - name: foo
    command: mcp-foo-v1

## worker

purpose: Handles background jobs
mcp_servers:
  - name: foo
    command: mcp-foo-v2
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path)
        .expect("kit should still be written despite the conflict");

    assert!(
        report.unmapped.iter().any(|l| l
            == "unmapped: mcp_server 'foo' (conflicting definitions across agents; kept first)"),
        "expected mcp_server conflict in unmapped, got {:?}",
        report.unmapped
    );

    let content = fs::read_to_string(&report.path).unwrap();
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("migrated kit must validate");
    let runtime = manifest.runtime.expect("runtime block required");
    assert_eq!(
        runtime.mcp_servers.len(),
        1,
        "conflicting definitions must still collapse to one entry (first wins)"
    );
    assert_eq!(
        runtime.mcp_servers[0].command.as_deref(),
        Some("mcp-foo-v1")
    );
}

/// Companion to the above: identical duplicate MCP server definitions
/// across sections must stay silent (not reported unmapped).
#[test]
fn identical_duplicate_mcp_servers_stay_silent() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md

## primary

purpose: Handles inbound requests
mcp_servers:
  - name: foo
    command: mcp-foo

## worker

purpose: Handles background jobs
mcp_servers:
  - name: foo
    command: mcp-foo
"#,
    )
    .unwrap();
    let out_path = dir.path().join("agent.swarmkit.yaml");

    let report = migrate_from_agents_md(&in_path, &out_path).expect("migration should succeed");
    assert!(
        report.unmapped.is_empty(),
        "identical duplicate mcp_server definitions must not be reported unmapped: {:?}",
        report.unmapped
    );

    let content = fs::read_to_string(&report.path).unwrap();
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("migrated kit must validate");
    let runtime = manifest.runtime.expect("runtime block required");
    assert_eq!(runtime.mcp_servers.len(), 1);
}

/// Regression fixture for the minimal single-agent AGENTS.md shape
/// (previously the `examples/01-hello-world/AGENTS.md` file on disk; inlined
/// here so this test doesn't depend on that example surviving the AGENTS.md
/// deprecation sweep).
#[test]
fn migrates_minimal_single_agent_file() {
    let dir = tempdir();
    let in_path = dir.path().join("AGENTS.md");
    fs::write(
        &in_path,
        r#"# AGENTS.md
## hello-agent
purpose: "A friendly agent that introduces itself and answers basic questions"
model: ministral-3b
mdns: true
"#,
    )
    .unwrap();
    let out_path = dir.path().join("hello.swarmkit.yaml");

    let report =
        migrate_from_agents_md(&in_path, &out_path).expect("fixture should migrate cleanly");
    assert!(
        report.unmapped.is_empty(),
        "minimal single-agent fixture should have no unmapped fields: {:?}",
        report.unmapped
    );

    let content = fs::read_to_string(&report.path).unwrap();
    let manifest = arkavo_swarmkit::parse_yaml(&content).expect("migrated kit must validate");
    assert_eq!(manifest.kit.id, report.kit_id);
    assert_eq!(manifest.roles[0].id, "hello-agent");
}
