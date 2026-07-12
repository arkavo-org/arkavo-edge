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

use super::agent::{AgentConfig, McpServerConfig, default_agent_name};
use super::kit::kit_model_to_hint;

/// Zero-config listen address: dynamic port, all interfaces. Matches the
/// legacy AGENTS.md-era default that used to live inline in `agent.rs`.
const DEFAULT_LISTEN: &str = "0.0.0.0:0";

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
    // `from_kit` distinguishes real kit roles from the zero-config default,
    // so a `-n` miss can report "no kit" instead of misleadingly presenting
    // the hostname-derived default name as a selectable role id.
    let (mut configs, from_kit) = match cli_config_path {
        // Explicit -c: errors (bad YAML, invalid kit) are fatal. No silent
        // fallback to defaults when the caller named a specific file.
        Some(explicit) => {
            let discovered = arkavo_swarmkit::load_kit_file(explicit)?;
            (agent_configs_from_kit(&discovered.config), true)
        }
        None => match arkavo_swarmkit::discover_kit_path(cwd) {
            Ok(path) => {
                let discovered = arkavo_swarmkit::load_kit_file(&path)?;
                (agent_configs_from_kit(&discovered.config), true)
            }
            // Only-AGENTS.md-present is non-fatal: log the migrate hint once
            // and fall through to the zero-config default. The AGENTS.md
            // content itself is never read.
            Err(err @ DiscoverError::AgentsMdUnsupported { .. }) => {
                eprintln!("{err}");
                (vec![default_agent_config()], false)
            }
            Err(DiscoverError::NotFound) => (vec![default_agent_config()], false),
            // Multiple candidates, or a read/parse failure during
            // discovery itself: fatal, with the error's own message.
            Err(err) => return Err(err.into()),
        },
    };

    if let Some(port) = port {
        for config in &mut configs {
            let host = config.listen.split(':').next().unwrap_or("0.0.0.0");
            config.listen = format!("{host}:{port}");
        }
    }

    let Some(name) = name else {
        return Ok(configs);
    };

    match configs.iter().position(|c| c.name == name) {
        Some(idx) => Ok(vec![configs.remove(idx)]),
        None if from_kit => {
            let available: Vec<&str> = configs.iter().map(|c| c.name.as_str()).collect();
            Err(format!(
                "no role {name:?} in kit; available role ids: {}",
                available.join(", ")
            )
            .into())
        }
        // Zero-config default: there is no kit, so there are no role ids to
        // offer — pointing at the default's hostname-derived name would
        // misleadingly imply a kit exists.
        None => Err(format!(
            "no SwarmKit manifest found, so there is no kit role {name:?} to select; \
             create one with 'arkavo kit init <name>' or pass -c <kit.swarmkit.yaml>"
        )
        .into()),
    }
}

/// Map every role in a loaded kit to an [`AgentConfig`]. Kit-level `runtime`
/// fields (`listen`, `mdns`, `mode`, `mcp_servers`) apply uniformly to every
/// role — a kit has exactly one `runtime` block, not one per role.
fn agent_configs_from_kit(runtime_config: &AgentRuntimeConfig) -> Vec<AgentConfig> {
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

    runtime_config
        .roles
        .iter()
        .map(|role| role_to_agent_config(role, &listen, mdns_enabled, mode.clone(), &mcp_servers))
        .collect()
}

fn role_to_agent_config(
    role: &RoleRuntimeView,
    listen: &str,
    mdns_enabled: bool,
    mode: AgentMode,
    mcp_servers: &[McpServerConfig],
) -> AgentConfig {
    let model = role
        .model_family
        .as_deref()
        .and_then(|family| kit_model_to_hint(family, role.model_size.as_deref()))
        .unwrap_or("")
        .to_string();

    AgentConfig {
        name: role.role_id.clone(),
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
    }
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

/// Zero-config default: no kit found anywhere in the resolution order.
/// Mirrors the construction that used to live inline at `agent.rs:198`.
fn default_agent_config() -> AgentConfig {
    AgentConfig {
        name: default_agent_name(),
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
    fn multiple_kits_in_cwd_is_fatal() {
        let dir = tempdir();
        let yaml = minimal_kit_yaml();
        fs::write(dir.path().join("a.swarmkit.yaml"), &yaml).unwrap();
        fs::write(dir.path().join("b.swarmkit.yaml"), &yaml).unwrap();

        let err = resolve_agent_configs(None, None, None, dir.path()).unwrap_err();
        assert!(err.to_string().contains("multiple"));
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
