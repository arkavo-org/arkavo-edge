//! SwarmKit-kit resolution for the `arkavo agent` run path (S6).
//!
//! `resolve_agent_configs` replaces AGENTS.md parsing at the CLI front door:
//! `-c/--config` loads an explicit kit file, otherwise
//! `arkavo_swarmkit::discover_kit_path` looks for one under the working
//! directory, and only the zero-config default falls back when no kit
//! exists anywhere. Product AGENTS.md is never read here — see
//! `arkavo_swarmkit::discover` for the rationale and the migrate hint.

use std::path::Path;

use arkavo_protocol::agent_config::AgentMode;
use arkavo_swarmkit::runtime_config::RoleRuntimeView;
use arkavo_swarmkit::{AgentRuntimeConfig, DiscoverError, RuntimeMcpServer, RuntimeMode};

use super::agent::listen::DEFAULT_LISTEN;
use super::agent::{AgentConfig, McpServerConfig, default_agent_name};
use super::kit::kit_model_to_hint;

/// Resolve the [`AgentConfig`](super::agent::AgentConfig)(s) to run for
/// `arkavo agent`, per the S6 resolution order: `-c` > discovery > the
/// zero-config default.
///
/// Returns one entry per kit role in manifest order — mirroring the
/// multi-agent output of the historical top-level AGENTS.md parser (deleted
/// in Task 14 / S6) — unless `name` narrows the
/// result to the single role whose id matches. `port`, when given, replaces
/// the port part of every returned entry's `listen` (current CLI flag
/// semantics, preserved). This function starts nothing; the caller (today,
/// `run_agent_with_options`) decides how many of the returned entries to
/// actually start.
pub fn resolve_agent_configs(
    cli_config_path: Option<&Path>,
    name: Option<&str>,
    port: Option<u16>,
    cwd: &Path,
) -> Result<Vec<AgentConfig>, Box<dyn std::error::Error>> {
    let kit = match cli_config_path {
        // Explicit -c: errors (bad YAML, invalid kit) are fatal. No silent
        // fallback to defaults when the caller named a specific file.
        Some(explicit) => Some(arkavo_swarmkit::load_kit_file(explicit)?),
        None => match arkavo_swarmkit::discover_kit_path(cwd) {
            Ok(path) => Some(arkavo_swarmkit::load_kit_file(&path)?),
            // Only-AGENTS.md-present is non-fatal: log the migrate hint once
            // and fall through to the zero-config default. The AGENTS.md
            // content itself is never read.
            Err(err @ DiscoverError::AgentsMdUnsupported { .. }) => {
                eprintln!("{err}");
                None
            }
            Err(DiscoverError::NotFound) => None,
            // Multiple candidates, or a read/parse failure during
            // discovery itself: fatal, with the error's own message.
            Err(err) => return Err(err.into()),
        },
    };

    let mut configs = match (&kit, name) {
        (Some(kit), _) => agent_configs_from_kit(&kit.config, name)?,
        // Zero-config default: there is no kit, so there are no role ids to
        // offer — pointing at the default's hostname-derived name would
        // misleadingly imply a kit exists.
        (None, Some(name)) => {
            return Err(format!(
                "no SwarmKit manifest found, so there is no kit role {name:?} to select; \
                 create one with 'arkavo kit init <name>' or pass -c <kit.swarmkit.yaml>"
            )
            .into());
        }
        (None, None) => vec![default_agent_config()],
    };

    if let Some(port) = port {
        for config in &mut configs {
            config.listen = listen_with_port(&config.listen, port);
        }
    }

    Ok(configs)
}

/// Replace the port of a `listen` address, keeping its host.
///
/// IP literals go through [`std::net::SocketAddr`] so an IPv6 host keeps its
/// brackets — splitting on the first `:` would reduce `[::]:8080` to `[`.
/// Hostnames only lose a trailing numeric `:port`, and an address that never
/// had a port keeps its whole host. An address with no host at all gets the
/// default's: `-p` picks a port, it does not widen where the agent listens.
fn listen_with_port(listen: &str, port: u16) -> String {
    use std::net::{IpAddr, SocketAddr};

    if let Ok(addr) = listen.parse::<SocketAddr>() {
        return SocketAddr::new(addr.ip(), port).to_string();
    }
    let unbracketed = listen.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        return SocketAddr::new(ip, port).to_string();
    }
    let host = match listen.rsplit_once(':') {
        Some((host, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => listen,
    };
    if host.is_empty() {
        return listen_with_port(DEFAULT_LISTEN, port);
    }
    format!("{host}:{port}")
}

/// Map the roles of a loaded kit to [`AgentConfig`]s: every role in manifest
/// order, or only the role `only_role` names. Kit-level `runtime` fields
/// (`listen`, `mdns`, `mode`, `mcp_servers`) apply uniformly to every
/// role — a kit has exactly one `runtime` block, not one per role.
///
/// Fails when a mapped role names a model the router does not know: the
/// model becomes the router hint (and, for a cloud arm, the operator's
/// consent to use it), so a typo must stop startup instead of quietly running
/// something else. Roles `only_role` leaves out are never started by this
/// process, so their models are not resolved — one role's typo must not keep
/// an unrelated role from starting.
fn agent_configs_from_kit(
    runtime_config: &AgentRuntimeConfig,
    only_role: Option<&str>,
) -> Result<Vec<AgentConfig>, String> {
    let roles: Vec<&RoleRuntimeView> = match only_role {
        Some(name) => {
            let Some(role) = runtime_config.roles.iter().find(|r| r.role_id == name) else {
                let available: Vec<&str> = runtime_config
                    .roles
                    .iter()
                    .map(|r| r.role_id.as_str())
                    .collect();
                return Err(format!(
                    "no role {name:?} in kit; available role ids: {}",
                    available.join(", ")
                ));
            };
            vec![role]
        }
        None => runtime_config.roles.iter().collect(),
    };

    let listen = runtime_config
        .runtime
        .listen
        .clone()
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let mdns_enabled = runtime_config.runtime.mdns_or_default();
    let mode = to_agent_mode(runtime_config.runtime.mode_or_default());
    let mcp_servers: Vec<McpServerConfig> = runtime_config
        .runtime
        .mcp_servers
        .iter()
        .map(to_mcp_server_config)
        .collect();

    roles
        .into_iter()
        .map(|role| role_to_agent_config(role, &listen, mdns_enabled, mode.clone(), &mcp_servers))
        .collect()
}

/// The router hint a role's declared model stands for; empty when the role
/// declares none, which leaves the choice to the router.
fn role_model_hint(role: &RoleRuntimeView) -> Result<String, String> {
    let Some(family) = role.model_family.as_deref() else {
        return Ok(String::new());
    };
    kit_model_to_hint(family, role.model_size.as_deref()).ok_or_else(|| {
        let declared = match role.model_size.as_deref() {
            Some(size) => format!("family {family:?}, size {size:?}"),
            None => format!("family {family:?}"),
        };
        format!(
            "role {:?}: model ({declared}) is not a model the router knows; name a local \
             edge model as family/size (e.g. ministral/3B) or a router model id as the \
             family with no size (e.g. gpt-6-astra)",
            role.role_id
        )
    })
}

fn role_to_agent_config(
    role: &RoleRuntimeView,
    listen: &str,
    mdns_enabled: bool,
    mode: AgentMode,
    mcp_servers: &[McpServerConfig],
) -> Result<AgentConfig, String> {
    let model = role_model_hint(role)?;

    Ok(AgentConfig {
        name: role.role_id.clone(),
        role_id: Some(role.role_id.clone()),
        purpose: role.skill_instructions.clone(),
        model,
        mode,
        listen: listen.to_string(),
        mdns_enabled,
        mcp_servers: mcp_servers.to_vec(),
        api_keys: std::collections::HashMap::new(),
        quiet: true,
        peers: Vec::new(),
        a2a_enabled: true,
        a2a_service_type: None,
        swarm: None,
    })
}

fn to_agent_mode(mode: RuntimeMode) -> AgentMode {
    match mode {
        RuntimeMode::Orchestrator => AgentMode::Orchestrator,
        RuntimeMode::Specialist => AgentMode::Specialist,
    }
}

fn to_mcp_server_config(s: &RuntimeMcpServer) -> McpServerConfig {
    McpServerConfig {
        name: s.name.clone(),
        command: s.command.clone(),
        args: s.args.clone(),
        url: s.url.clone(),
    }
}

/// Resolve just the kit *path* SwarmKit would use for `-c`/discovery.
///
/// Does not load or parse it. Mirrors the resolution order at the top of
/// [`resolve_agent_configs`] (`-c` > `discover_kit_path`), but stays pure —
/// no filesystem writes, no env mutation — so it is safe to call from unit
/// tests and from [`export_resolved_kit_path`] alike.
///
/// Returns `None` only for the zero-config-default case (no kit anywhere):
/// there is nothing to export to `ARKAVO_SWARMKIT_PATH` then.
pub fn resolve_kit_path(cli_config_path: Option<&Path>, cwd: &Path) -> Option<std::path::PathBuf> {
    match cli_config_path {
        Some(explicit) => Some(if explicit.is_absolute() {
            explicit.to_path_buf()
        } else {
            cwd.join(explicit)
        }),
        None => arkavo_swarmkit::discover_kit_path(cwd).ok(),
    }
}

/// Export the resolved kit path to `ARKAVO_SWARMKIT_PATH` for the rest of
/// this process.
///
/// Server-side policy loaders (`arkavo_router::load_agent_config`, the
/// spend plane, agui) re-discover their own kit from process cwd/env and
/// never see an explicit `-c` path passed to the CLI — without this, a
/// kit's preflight/KAS/budget policy is silently not applied unless the
/// same kit also happens to be cwd-discoverable. `discover_kit_path`
/// already prefers this env var over directory scanning, so setting it to
/// the path this same resolution just landed on is idempotent when nothing
/// was set before; when a stale value *was* set, this overwrites it so
/// every consumer in the process agrees with what the CLI just resolved.
///
/// No-op when no kit was resolved (zero-config default): there is nothing
/// to export, and clearing a pre-existing env var here would be a
/// surprising side effect of an agent run that has no kit at all.
pub fn export_resolved_kit_path(cli_config_path: Option<&Path>, cwd: &Path) {
    if let Some(path) = resolve_kit_path(cli_config_path, cwd) {
        // SAFETY: called once, early, from the single-threaded CLI startup
        // path (`run_agent_with_options`), strictly before the A2A server
        // (the first reader of this var) is started.
        unsafe {
            std::env::set_var(arkavo_swarmkit::SWARMKIT_PATH_ENV, path);
        }
    }
}

/// Zero-config default: no kit found anywhere in the resolution order.
fn default_agent_config() -> AgentConfig {
    AgentConfig {
        name: default_agent_name(),
        role_id: None,
        purpose: "A general-purpose AI agent".to_string(),
        model: String::new(),
        mode: AgentMode::default(),
        listen: DEFAULT_LISTEN.to_string(),
        mdns_enabled: true,
        mcp_servers: Vec::new(),
        api_keys: std::collections::HashMap::new(),
        quiet: true,
        peers: Vec::new(),
        a2a_enabled: true,
        a2a_service_type: None,
        swarm: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    #[test]
    fn not_found_and_no_config_returns_single_default() {
        let dir = tempdir();
        let configs = resolve_agent_configs(None, None, None, dir.path()).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].listen, DEFAULT_LISTEN);
    }

    #[test]
    fn listen_with_port_replaces_ipv4_port() {
        assert_eq!(listen_with_port("0.0.0.0:0", 8080), "0.0.0.0:8080");
        assert_eq!(listen_with_port("127.0.0.1:9000", 8080), "127.0.0.1:8080");
    }

    #[test]
    fn listen_with_port_keeps_bracketed_ipv6_host() {
        // Regression: splitting on the first ':' turned `[::]:8080` into `[:9090`.
        assert_eq!(listen_with_port("[::]:8080", 9090), "[::]:9090");
        assert_eq!(listen_with_port("[::1]:0", 9090), "[::1]:9090");
        assert_eq!(listen_with_port("[fe80::1]", 9090), "[fe80::1]:9090");
        assert_eq!(listen_with_port("::1", 9090), "[::1]:9090");
    }

    #[test]
    fn listen_with_port_handles_hostnames_with_and_without_port() {
        assert_eq!(listen_with_port("localhost", 8080), "localhost:8080");
        assert_eq!(listen_with_port("localhost:3000", 8080), "localhost:8080");
    }

    /// Regression: an address with no host was given `0.0.0.0`, so `-p`
    /// alone could move an agent onto every interface.
    #[test]
    fn listen_with_port_gives_a_missing_host_the_loopback_default() {
        assert_eq!(listen_with_port("", 8080), "127.0.0.1:8080");
        assert_eq!(listen_with_port(":3000", 8080), "127.0.0.1:8080");
    }

    /// Regression: a kit with no `runtime.listen`, and the zero-config
    /// default, listened on all interfaces.
    #[test]
    fn a_kit_without_runtime_listen_listens_on_loopback() {
        let dir = tempdir();
        let path = dir.path().join("agent.swarmkit.yaml");
        fs::write(&path, minimal_kit_yaml()).unwrap();

        let configs = resolve_agent_configs(Some(&path), None, None, dir.path()).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].listen, "127.0.0.1:0");

        let with_port = resolve_agent_configs(Some(&path), None, Some(8343), dir.path()).unwrap();
        assert_eq!(with_port[0].listen, "127.0.0.1:8343");
    }

    /// The role id travels with the configuration so the server can re-read
    /// the same role when the kit is reloaded.
    #[test]
    fn a_kit_role_carries_its_role_id_and_the_default_has_none() {
        let dir = tempdir();
        let path = dir.path().join("agent.swarmkit.yaml");
        fs::write(&path, minimal_kit_yaml()).unwrap();

        let configs = resolve_agent_configs(Some(&path), None, None, dir.path()).unwrap();
        assert_eq!(configs[0].role_id.as_deref(), Some("agent"));

        assert_eq!(default_agent_config().role_id, None);
    }

    #[test]
    fn a_kit_that_names_another_interface_is_honoured() {
        let dir = tempdir();
        let kit =
            minimal_kit_yaml().replacen("kit:", "runtime:\n  listen: \"0.0.0.0:8342\"\nkit:", 1);
        let path = dir.path().join("agent.swarmkit.yaml");
        fs::write(&path, kit).unwrap();

        let configs = resolve_agent_configs(Some(&path), None, None, dir.path()).unwrap();
        assert_eq!(configs[0].listen, "0.0.0.0:8342");
    }

    #[test]
    fn port_override_preserves_ipv6_listen_from_kit() {
        let dir = tempdir();
        let kit = minimal_kit_yaml().replacen("kit:", "runtime:\n  listen: \"[::]:8080\"\nkit:", 1);
        let path = dir.path().join("agent.swarmkit.yaml");
        fs::write(&path, kit).unwrap();

        let configs = resolve_agent_configs(Some(&path), None, Some(9090), dir.path()).unwrap();
        assert!(!configs.is_empty());
        for config in configs {
            assert_eq!(config.listen, "[::]:9090");
        }
    }

    #[test]
    fn multiple_kits_in_cwd_is_fatal() {
        let dir = tempdir();
        let yaml = minimal_kit_yaml();
        fs::write(dir.path().join("a.swarmkit.yaml"), &yaml).unwrap();
        fs::write(dir.path().join("b.swarmkit.yaml"), &yaml).unwrap();

        let err = resolve_agent_configs(None, None, None, dir.path()).unwrap_err();
        assert!(err.to_string().contains("multiple"));
    }

    // Finding 1: -c kit's preflight/KAS/budget silently not applied unless
    // the kit is also cwd-discoverable. `resolve_kit_path` is the pure seam
    // `export_resolved_kit_path` builds on; these tests exercise it without
    // touching process env (see `agent_kit_resolution_test.rs` for the
    // actual env-mutation integration test).
    #[test]
    fn resolve_kit_path_explicit_relative_joins_cwd() {
        let dir = tempdir();
        let explicit = Path::new("some/kit.swarmkit.yaml");
        let resolved = resolve_kit_path(Some(explicit), dir.path()).expect("explicit is Some");
        assert_eq!(resolved, dir.path().join(explicit));
    }

    #[test]
    fn resolve_kit_path_explicit_absolute_stays_absolute() {
        let dir = tempdir();
        let explicit = dir.path().join("kit.swarmkit.yaml");
        let resolved =
            resolve_kit_path(Some(&explicit), Path::new("/somewhere/else")).expect("is Some");
        assert_eq!(resolved, explicit);
    }

    #[test]
    fn resolve_kit_path_discovery_finds_kit_under_dot_arkavo() {
        let dir = tempdir();
        let arkavo_dir = dir.path().join(".arkavo");
        fs::create_dir_all(&arkavo_dir).unwrap();
        let kit_path = arkavo_dir.join("agent.swarmkit.yaml");
        fs::write(&kit_path, minimal_kit_yaml()).unwrap();

        let resolved = resolve_kit_path(None, dir.path()).expect("discovery should find the kit");
        assert_eq!(resolved, kit_path);
    }

    #[test]
    fn resolve_kit_path_no_kit_anywhere_is_none() {
        let dir = tempdir();
        assert_eq!(resolve_kit_path(None, dir.path()), None);
    }

    fn minimal_kit_yaml() -> String {
        r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
    skills: []
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
"#
        .to_string()
    }
}
