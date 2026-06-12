//! `arkavo swarmkit play <kit>` — run a SwarmKit as a live agent flight by
//! mapping each role to the existing agent runtime (`start_agent_server`).
//!
//! This module currently provides only the pure mapping function
//! [`kit_to_agent_configs`]; the command/dispatch layer is a separate task.

use crate::commands::agent::{AgentConfig, McpServerConfig};
use arkavo_protocol::agent_config::{AgentMode, expand_env};
use arkavo_swarmkit::{McpServerDef, manifest::Manifest};

/// First listen port assigned to role index 0; subsequent roles get
/// consecutive ports. Deterministic so peer wiring is reproducible.
const BASE_PORT: u16 = 8450;

/// Build the per-role agent configs for a parsed, validated kit.
///
/// Index order is the manifest role order, which keeps port assignment
/// deterministic. A role with one or more `mcp_tools` grants becomes an
/// [`AgentMode::Orchestrator`] (hub); all other roles are
/// [`AgentMode::Specialist`] spokes that peer back to the hub.
pub fn kit_to_agent_configs(manifest: &Manifest) -> Vec<AgentConfig> {
    let servers: std::collections::HashMap<&str, &McpServerDef> = manifest
        .mcp_servers
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();

    let listens: Vec<String> = manifest
        .roles
        .iter()
        .enumerate()
        .map(|(i, _)| format!("0.0.0.0:{}", BASE_PORT + i as u16))
        .collect();

    // The hub is the first role holding an MCP grant. Spokes peer to it.
    let hub_idx = manifest.roles.iter().position(|r| !r.mcp_tools.is_empty());

    let port_of = |listen: &str| listen.rsplit(':').next().unwrap_or("").to_string();

    manifest
        .roles
        .iter()
        .enumerate()
        .map(|(i, role)| {
            let has_grant = !role.mcp_tools.is_empty();

            let mcp_servers: Vec<McpServerConfig> = role
                .mcp_tools
                .iter()
                .filter_map(|grant| {
                    servers
                        .get(grant.server.as_str())
                        .map(|def| McpServerConfig {
                            name: def.name.clone(),
                            command: Some(expand_env(&def.command)),
                            args: def.args.iter().map(|a| expand_env(a)).collect(),
                            url: None,
                        })
                })
                .collect();

            // Hub peers to every spoke; each spoke peers only to the hub.
            let peers: Vec<String> = if has_grant {
                listens
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, l)| format!("http://localhost:{}", port_of(l)))
                    .collect()
            } else if let Some(h) = hub_idx {
                vec![format!("http://localhost:{}", port_of(&listens[h]))]
            } else {
                vec![]
            };

            // Inline-skill instructions become the agent's system prompt.
            // `Skill.payload` is a free-form JSON `Value`; the SkillContent
            // shape exposes the prompt under `instructions`.
            let purpose = role
                .skills
                .iter()
                .find_map(|s| {
                    s.payload
                        .as_ref()
                        .and_then(|p| p.get("instructions"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("Role: {}", role.role_type));

            // `agent_provisioning` is always present; `model` is optional.
            let model = role
                .agent_provisioning
                .model
                .as_ref()
                .map(|m| match &m.size {
                    Some(sz) => format!("{}-{}", m.family, sz),
                    None => m.family.clone(),
                })
                .unwrap_or_default();

            AgentConfig {
                name: role.id.clone(),
                purpose,
                model,
                mode: if has_grant {
                    AgentMode::Orchestrator
                } else {
                    AgentMode::Specialist
                },
                listen: listens[i].clone(),
                mdns_enabled: true,
                mcp_servers,
                api_keys: std::collections::HashMap::new(),
                quiet: true,
                peers,
                a2a_enabled: true,
                a2a_service_type: None,
                swarm: Some(slug(&manifest.kit.name)),
            }
        })
        .collect()
}

/// Lowercase, hyphenate non-alphanumerics, and trim leading/trailing hyphens.
/// Used to derive a stable swarm identifier from the kit name.
fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// `arkavo swarmkit play <kit.yaml> [--role <id>]`
#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut kit_path: Option<String> = None;
    let mut only_role: Option<String> = None;
    let mut sub: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "play" => sub = Some("play"),
            "--role" if i + 1 < args.len() => {
                only_role = Some(args[i + 1].clone());
                i += 1;
            }
            "-h" | "--help" | "help" => {
                print_usage();
                return Ok(());
            }
            p if !p.starts_with('-') && kit_path.is_none() => kit_path = Some(p.to_string()),
            other => return Err(format!("Unknown swarmkit arg '{other}'").into()),
        }
        i += 1;
    }
    if sub != Some("play") {
        print_usage();
        return Err("expected: arkavo swarmkit play <kit.yaml>".into());
    }
    let kit_path = kit_path.ok_or("missing <kit.yaml> path")?;

    let yaml = std::fs::read_to_string(&kit_path)
        .map_err(|e| format!("cannot read kit '{kit_path}': {e}"))?;
    let manifest =
        arkavo_swarmkit::parse_yaml(&yaml).map_err(|e| format!("kit parse error: {e}"))?;
    arkavo_swarmkit::validate(&manifest).map_err(|e| format!("kit invalid: {e}"))?;

    let mut configs = kit_to_agent_configs(&manifest);
    if let Some(role) = &only_role {
        configs.retain(|c| &c.name == role);
        if configs.is_empty() {
            return Err(format!("no role named '{role}' in kit").into());
        }
    }

    println!(
        "[swarmkit] {} v{} — launching {} role(s): {}",
        manifest.kit.name,
        manifest.kit.version,
        configs.len(),
        configs
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // `start_agent_server`'s future is not `Send` (it holds a `Box<dyn Error>`
    // across an await), so we can't `tokio::spawn` it. Instead we run every
    // role's agent loop concurrently on the single `block_on` thread via
    // `join_all`, which has no `Send` requirement.
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let tasks = configs.into_iter().map(|cfg| async move {
            let name = cfg.name.clone();
            if let Err(e) = crate::commands::agent::start_agent_server(&cfg, false).await {
                eprintln!("[swarmkit] role '{name}' exited: {e}");
            }
        });
        futures::future::join_all(tasks).await;
    });
    Ok(())
}

fn print_usage() {
    eprintln!("Usage: arkavo swarmkit play <kit.yaml> [--role <id>]");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_grant_role_to_orchestrator_with_mcp_server() {
        let yaml = include_str!("../../../arkavo-swarmkit/tests/fixtures/declared_mcp_server.yaml");
        let manifest = arkavo_swarmkit::parse_yaml(yaml).unwrap();
        let configs = kit_to_agent_configs(&manifest);

        assert_eq!(configs.len(), manifest.roles.len());

        let survivor = configs.iter().find(|c| c.name == "survivor").unwrap();
        // A role with an mcp_tools grant is the hub/orchestrator.
        assert_eq!(survivor.mode, AgentMode::Orchestrator);
        // Role index 0 -> BASE_PORT.
        assert_eq!(survivor.listen, "0.0.0.0:8450");
        // The grant resolves to the declared `game-rl` MCP server.
        assert_eq!(survivor.mcp_servers.len(), 1);
        assert_eq!(survivor.mcp_servers[0].name, "game-rl");
        // ${VAR} stays literal when unset, so the command is visible downstream.
        assert_eq!(
            survivor.mcp_servers[0].command.as_deref(),
            Some("${GAME_RL_SERVER}")
        );
        // No inline skill -> purpose falls back to the role_type label.
        assert_eq!(survivor.purpose, "Role: specialist");
        // swarm identifier is the slugified kit name.
        assert_eq!(survivor.swarm.as_deref(), Some("declared-mcp-server-kit"));
    }
}
