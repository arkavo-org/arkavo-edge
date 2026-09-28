//! `arkavo swarmkit play <kit>` — run a SwarmKit as a live agent flight by
//! mapping each role to the existing agent runtime (`start_agent_server`).

use crate::commands::agent::{AgentConfig, McpServerConfig};
use arkavo_protocol::agent_config::AgentMode;
use arkavo_swarmkit::{McpServerDef, manifest::Manifest};
use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use tokio::sync::watch;

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
                            // ${VAR} is expanded once at the spawn site
                            // (McpClient::new_with_command); do not pre-expand here.
                            name: def.name.clone(),
                            command: Some(def.command.clone()),
                            args: def.args.clone(),
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
        // A single isolated role has no live peers to reach; clear the
        // full-topology peer wiring so it doesn't spin retrying absent A2A peers.
        for c in &mut configs {
            c.peers.clear();
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

    // The CLI agent runtime (AgentConfig) does not yet carry per-role budgets,
    // inference params, context limits, or completion.rules. Warn loudly rather
    // than dropping them silently (tracked: swarmkit play provisioning passthrough).
    let mut unenforced: Vec<&str> = Vec::new();
    if manifest
        .roles
        .iter()
        .any(|r| r.agent_provisioning.budget.is_some())
    {
        unenforced.push("agent_provisioning.budget");
    }
    if manifest
        .roles
        .iter()
        .any(|r| r.agent_provisioning.inference.is_some())
    {
        unenforced.push("agent_provisioning.inference");
    }
    if manifest
        .roles
        .iter()
        .any(|r| r.agent_provisioning.context.is_some())
    {
        unenforced.push("agent_provisioning.context");
    }
    // `completion` is a required `CompletionSpec`; its `rules` is a `Vec<String>`.
    // Treat a non-empty rule set as a declared-but-unenforced control.
    if !manifest.completion.rules.is_empty() {
        unenforced.push("completion.rules");
    }
    if !unenforced.is_empty() {
        eprintln!(
            "[swarmkit] note: these kit fields are NOT yet enforced by `swarmkit play` and are ignored this run: {}",
            unenforced.join(", ")
        );
    }

    supervise(configs, |cfg, mut shutdown| async move {
        let stop = async move {
            // An Err here means the supervisor dropped the sender, which only
            // happens once it is done; stopping is the right response either way.
            let _ = shutdown.wait_for(|stop| *stop).await;
        };
        crate::commands::agent::start_agent_server_until(&cfg, false, stop).await
    })
    .map_err(Into::into)
}

/// Run each role on its own OS thread and supervise them as one swarm.
///
/// `start_agent_server`'s future is `!Send` (it holds a `Box<dyn Error>`
/// across an await), so roles cannot be `tokio::spawn`ed onto shared workers;
/// each gets its own thread and current-thread runtime so blocking work in
/// one role (e.g. local inference) does not stall the others.
///
/// The first role to stop, whether by error, panic, Ctrl-C or a clean return,
/// flips the shared shutdown signal so its siblings tear down too. Without
/// that, a crashed hub would leave the advisors blocked on Ctrl-C and the
/// command would hang instead of exiting non-zero. Returns the first failure.
fn supervise<F, Fut>(configs: Vec<AgentConfig>, run: F) -> Result<(), String>
where
    F: Fn(AgentConfig, watch::Receiver<bool>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = mpsc::channel::<(String, Result<(), String>)>();
    let mut handles = Vec::new();
    for cfg in configs {
        let name = cfg.name.clone();
        let (run, shutdown, done) = (run.clone(), shutdown_rx.clone(), done_tx.clone());
        handles.push(std::thread::spawn(move || {
            // Catch the panic here so a crashed role still reports in and
            // counts as a failure rather than vanishing from the tally.
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| run_role(cfg, shutdown, run)))
                .unwrap_or_else(|_| Err("panicked".to_string()));
            // Fails only if the supervisor is gone, and then nobody is listening.
            let _ = done.send((name, outcome));
        }));
    }
    // Drop our sender so the loop below ends once every role has reported.
    drop(done_tx);

    let mut first_failure = None;
    for (name, outcome) in done_rx {
        shutdown_tx.send_replace(true);
        if let Err(e) = outcome {
            eprintln!("[swarmkit] role '{name}' exited: {e}");
            first_failure.get_or_insert_with(|| format!("role '{name}' failed: {e}"));
        }
    }
    for handle in handles {
        // Every role already reported (panics included), so this cannot block
        // or carry a new failure; it just reaps the threads.
        let _ = handle.join();
    }
    first_failure.map_or(Ok(()), Err)
}

/// Owns a fresh runtime on the role's dedicated thread, so no outer async
/// context exists for `block_on` to deadlock against.
#[allow(clippy::disallowed_methods)]
fn run_role<F, Fut>(cfg: AgentConfig, shutdown: watch::Receiver<bool>, run: F) -> Result<(), String>
where
    F: Fn(AgentConfig, watch::Receiver<bool>) -> Fut,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime init failed: {e}"))?;
    rt.block_on(run(cfg, shutdown)).map_err(|e| e.to_string())
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

    /// Copies of the fixture role under the given names, standing in for a hub
    /// plus advisors; `supervise` only cares about names.
    fn swarm_configs(names: &[&str]) -> Vec<AgentConfig> {
        let yaml = include_str!("../../../arkavo-swarmkit/tests/fixtures/declared_mcp_server.yaml");
        let base = kit_to_agent_configs(&arkavo_swarmkit::parse_yaml(yaml).unwrap()).remove(0);
        names
            .iter()
            .map(|n| AgentConfig {
                name: (*n).to_string(),
                ..base.clone()
            })
            .collect()
    }

    /// Runs `supervise` off-thread and fails the test instead of hanging it.
    fn supervise_within<F, Fut>(configs: Vec<AgentConfig>, run: F) -> Result<(), String>
    where
        F: Fn(AgentConfig, watch::Receiver<bool>) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error>>>,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(supervise(configs, run)));
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("swarm hung after a role stopped instead of tearing down")
    }

    #[test]
    fn failing_role_tears_down_siblings_and_returns_error() {
        let configs = swarm_configs(&["hub", "advisor-a", "advisor-b"]);
        let result = supervise_within(configs, |cfg, mut shutdown| async move {
            if cfg.name == "hub" {
                return Err("bind failed".into());
            }
            // Like a live agent, siblings only stop when told to.
            let _ = shutdown.wait_for(|stop| *stop).await;
            Ok(())
        });
        assert_eq!(result.unwrap_err(), "role 'hub' failed: bind failed");
    }

    #[test]
    fn panicking_role_tears_down_siblings_and_returns_error() {
        let configs = swarm_configs(&["hub", "advisor"]);
        let result = supervise_within(configs, |cfg, mut shutdown| async move {
            assert_ne!(cfg.name, "advisor", "advisor role crashed");
            let _ = shutdown.wait_for(|stop| *stop).await;
            Ok(())
        });
        assert_eq!(result.unwrap_err(), "role 'advisor' failed: panicked");
    }

    #[test]
    fn clean_role_exit_stops_swarm_without_error() {
        let configs = swarm_configs(&["hub", "advisor"]);
        let result = supervise_within(configs, |cfg, mut shutdown| async move {
            if cfg.name != "hub" {
                let _ = shutdown.wait_for(|stop| *stop).await;
            }
            Ok(())
        });
        assert_eq!(result, Ok(()));
    }
}
