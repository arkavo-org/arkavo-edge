use arkavo_config_encryption::AgentCredential;
#[cfg(feature = "mdns")]
use arkavo_protocol::get_service_ip;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

mod advertise;
mod gpu_residency;
pub mod listen;

/// What a command line asks `arkavo agent` to do.
#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Help,
    Init { name: Option<String> },
    Run(RunOptions),
}

/// The options of an `arkavo agent` run, as parsed from the command line.
///
/// `trust` and `bind` are separate on purpose: `--trust` only shows the
/// authorization QR code, and where the agent listens is `--bind`'s alone.
#[derive(Debug, Default, PartialEq, Eq)]
struct RunOptions {
    config_path: Option<String>,
    verbose: bool,
    trust: bool,
    bind: Option<listen::BindAddress>,
    port: Option<u16>,
    name: Option<String>,
}

/// Read the command line. Nothing is resolved or started here, so a
/// mistake in it is reported before the agent has touched anything.
#[allow(clippy::disallowed_methods)]
fn parse_args(args: &[String]) -> Result<Invocation, Box<dyn std::error::Error>> {
    let mut options = RunOptions::default();
    let mut subcommand: Option<&str> = None;
    let mut init_name: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-c" | "--config" => {
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    options.config_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-p" | "--port" => {
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    if let Ok(port) = args[i + 1].parse::<u16>() {
                        options.port = Some(port);
                    } else {
                        eprintln!("Error: Invalid port number '{}'", args[i + 1]);
                        return Err("Invalid port number".into());
                    }
                    i += 1;
                }
            }
            "-n" | "--name" => {
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    options.name = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            // A `--bind` that cannot be understood is a mistake in the
            // command line, not something to fall back from. The error is
            // returned, not printed here as well: the caller prints it once.
            "--bind" => match args.get(i + 1).filter(|value| !value.starts_with('-')) {
                Some(value) => {
                    options.bind = Some(listen::parse_bind(value)?);
                    i += 1;
                }
                None => {
                    return Err("--bind requires an address, for example --bind 127.0.0.1".into());
                }
            },
            "-v" | "--verbose" => options.verbose = true,
            "--trust" => options.trust = true,
            "-h" | "--help" | "help" => return Ok(Invocation::Help),
            "init" => {
                subcommand = Some("init");
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    init_name = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "run" => subcommand = Some("run"),
            // An unrecognized option must surface an error rather than silently booting
            // the agent — e.g. a typo like `arkavo --trsut` (also reachable via the bare
            // `arkavo <flag>` top-level route, which dispatches here).
            unknown if unknown.starts_with('-') => {
                eprintln!("Error: Unknown option '{unknown}'");
                print_usage();
                return Err(format!("Unknown option: {unknown}").into());
            }
            _ => {
                // Unknown non-dash token: an unrecognized subcommand.
                if subcommand.is_none() {
                    eprintln!("Error: Unknown agent subcommand '{arg}'");
                    print_usage();
                    return Err(format!("Unknown subcommand: {arg}").into());
                }
            }
        }
        i += 1;
    }

    Ok(match subcommand {
        Some("init") => Invocation::Init { name: init_name },
        _ => Invocation::Run(options),
    })
}

#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match parse_args(args)? {
        Invocation::Help => {
            print_usage();
            Ok(())
        }
        Invocation::Init { name: Some(name) } => {
            let report = deprecated_init(Path::new("."), &name)?;
            println!("Wrote {}", report.path.display());
            println!("kit.id: {}", report.kit_id);
            Ok(())
        }
        Invocation::Init { name: None } => {
            eprintln!("Error: Agent name required");
            eprintln!("Usage: arkavo agent init <agent-name>");
            Err("Missing agent name".into())
        }
        Invocation::Run(options) => run_agent_with_options(&options),
    }
}

fn print_usage() {
    println!("{USAGE}");
}

const USAGE: &str = r#"Arkavo Agent - Configure and run AI agents

USAGE:
    arkavo agent [OPTIONS]
    arkavo agent init <name>

SUBCOMMANDS:
    init <name>         [DEPRECATED] alias for 'arkavo kit init <name>'; writes .arkavo/<name>.swarmkit.yaml
    run                 Run an agent (alias for default behavior)
    help                Print this help message

OPTIONS:
    -c, --config <FILE> SwarmKit manifest path (default: discover .arkavo/*.swarmkit.yaml or ./*.swarmkit.yaml)
    -p, --port <PORT>   Override the listen port (default: random available port)
    --bind <ADDRESS>    Listen on this IP address instead of the default or the
                        kit's runtime.listen: 127.0.0.1, [::1] or 0.0.0.0, with an
                        optional port (127.0.0.1:8342). Without a port, -p or the
                        kit's port applies
    -n, --name <NAME>   Select a role by id from a multi-role kit (default: the first role)
    -v, --verbose       Show startup messages and status
    --trust             Show the agent authorization QR code (DID:key) on startup

NETWORK:
    By default the agent listens on loopback and announces itself over mDNS
    for discovery on this machine. The RPC endpoint is not authenticated yet.
    To accept connections from other machines, choose an explicit address with
    --bind 0.0.0.0 or runtime.listen. A network-reachable start prints a notice:
    run the agent on networks you trust.
    --bind 127.0.0.1 keeps the agent on this machine, whatever the kit says;
    agents on the same machine still discover it. A kit can pin an address with
    runtime.listen, for example runtime.listen: "127.0.0.1:8342". --bind
    overrides it, and says so.

EXAMPLES:
    arkavo agent                           # Run with auto-discovery
    arkavo agent --config agent.swarmkit.yaml  # Run with a specific kit
    arkavo agent --port 8343 -v            # Run on specific port with verbose
    arkavo agent -c team.swarmkit.yaml -n worker -p 8343  # Run one role of a multi-role kit
    arkavo agent --bind 127.0.0.1          # Stay on this machine
    arkavo agent run --bind 0.0.0.0 --trust # Expose the agent and show its QR code"#;

/// Deprecated: `arkavo agent init` no longer writes AGENTS.md.
///
/// It now prints a deprecation warning and delegates entirely to the same
/// manifest writer `kit init` uses, so the two commands can never drift in
/// what they produce. `base_dir` is exposed (rather than hardcoded to `.`)
/// so tests can drive this without touching the process's current
/// directory; the CLI arm always passes `.`.
pub fn deprecated_init(
    base_dir: &Path,
    name: &str,
) -> Result<crate::commands::kit::KitInitReport, Box<dyn std::error::Error>> {
    eprintln!(
        "warning: 'arkavo agent init' is deprecated; use 'arkavo kit init <name>'. Writing a SwarmKit manifest."
    );
    crate::commands::kit::init_kit(base_dir, name)
}

/// Generate the zero-config default agent name: `<hostname>-<folder>`, e.g.
/// `macbook-arkavo-edge`. Used by the SwarmKit-kit resolution path
/// (`agent_kit::resolve_agent_configs`) when no kit is found anywhere in the
/// resolution order.
pub(crate) fn default_agent_name() -> String {
    use std::process::Command;

    // Get machine hostname (strip .local suffix if present)
    let hostname = Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().trim_end_matches(".local").to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Get current folder name
    let folder_name = std::env::current_dir()
        .ok()
        .and_then(|path| path.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    format!("{hostname}-{folder_name}")
}

/// What `arkavo agent` starts with: the agent, the address its `listen`
/// parses to, and the line to show when `--bind` set aside the listen
/// address the kit asked for.
type StartupConfig = (AgentConfig, std::net::SocketAddr, Option<String>);

/// Resolve the single agent this process will run, and the address it will
/// listen on. With `bind` that address is the one `--bind` names.
///
/// Mirrors legacy multi-agent behavior by only ever starting the first
/// resolved entry, unless -n/--name narrowed the result to one role.
///
/// The listen address is parsed here, before anything is started or
/// exported, so a kit whose `runtime.listen` cannot be understood stops the
/// run with nothing bound.
fn resolve_startup_config(
    cli_config_path: Option<&Path>,
    name: Option<&str>,
    port: Option<u16>,
    bind: Option<listen::BindAddress>,
    cwd: &Path,
) -> Result<StartupConfig, Box<dyn std::error::Error>> {
    use crate::commands::agent_kit::resolve_agent_configs_for_start;

    let (configs, bind_notice) =
        resolve_agent_configs_for_start(cli_config_path, name, port, bind, cwd)?;
    let agent = configs
        .into_iter()
        .next()
        .ok_or("No agent configuration available")?;
    let listen_addr = listen::parse_listen(&agent.listen)?;
    Ok((agent, listen_addr, bind_notice))
}

#[allow(clippy::disallowed_methods)]
fn run_agent_with_options(options: &RunOptions) -> Result<(), Box<dyn std::error::Error>> {
    use crate::commands::agent;
    use crate::commands::agent_kit::export_resolved_kit_path;

    let cwd = std::env::current_dir()?;
    let cli_config_path = options.config_path.as_deref().map(Path::new);
    let verbose = options.verbose;
    let trust = options.trust;

    // Resolve config from a SwarmKit kit: -c/--config > discovery > the
    // zero-config default. AGENTS.md is never read on this path (S6).
    let (mut agent_config, _listen_addr, bind_notice) = resolve_startup_config(
        cli_config_path,
        options.name.as_deref(),
        options.port,
        options.bind,
        &cwd,
    )?;

    // Shown in a quiet run too: the kit asked for an address and the agent
    // is not on it.
    if let Some(notice) = bind_notice {
        eprintln!("{notice}");
    }

    // Export the resolved kit path so server-side policy loaders (preflight,
    // budget, KAS — which re-discover their own kit from process cwd/env)
    // see the same kit this just resolved, even when it came from an
    // explicit -c path that isn't itself cwd-discoverable.
    export_resolved_kit_path(cli_config_path, &cwd);

    gpu_residency::release_gpu_memory_when_idle();

    // Set verbose mode - default is quiet (verbose = false)
    agent_config.quiet = !verbose;

    if verbose {
        println!("Starting agent: {}", agent_config.name);
    }

    // A failed start is final. Retrying with a different configuration would
    // put the agent on an address and persona the operator never asked for.
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.block_on(async { agent::start_agent_server(&agent_config, trust).await })
        }
        Err(_) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async { agent::start_agent_server(&agent_config, trust).await })
        }
    }
}

// Runtime agent identity. Built from a SwarmKit kit at run time (`agent_kit`); the
// AGENTS.md migration parser (`kit::agents_md`) produces the same shape so it can map
// every legacy field onto the kit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub name: String,
    /// Id of the kit role this configuration was built from. `None` for the
    /// zero-config default and for AGENTS.md migration input, which have no
    /// kit role behind them.
    pub role_id: Option<String>,
    /// Short description of what the agent does, from the kit role's
    /// `description`. This is what the agent publishes about itself (agent
    /// card, mDNS); `purpose` is never published.
    pub description: Option<String>,
    pub purpose: String, // Used as system prompt for LLM
    pub model: String,
    pub mode: arkavo_protocol::agent_config::AgentMode,
    pub listen: String,
    pub mdns_enabled: bool,
    pub mcp_servers: Vec<McpServerConfig>,
    pub api_keys: std::collections::HashMap<String, String>,
    pub quiet: bool, // Default true (quiet), false if --verbose is specified
    // A2A peer configuration
    pub peers: Vec<String>,               // e.g., ["http://localhost:8352"]
    pub a2a_enabled: bool,                // Default: true
    pub a2a_service_type: Option<String>, // Custom mDNS service type
    pub swarm: Option<String>,            // Domain/swarm identifier for learning isolation
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            role_id: None,
            description: None,
            purpose: String::new(),
            model: String::new(),
            mode: arkavo_protocol::agent_config::AgentMode::default(),
            listen: listen::DEFAULT_LISTEN.to_string(),
            mdns_enabled: true, // Zero-config discovery
            mcp_servers: Vec::new(),
            api_keys: std::collections::HashMap::new(),
            quiet: true,
            peers: Vec::new(),
            a2a_enabled: true,
            a2a_service_type: None,
            swarm: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    /// What the server process may see of the agent's environment beyond
    /// the platform baseline.
    pub env: arkavo_process_env::EnvSpec,
}

/// Check if a tool's input schema has required arguments
fn has_required_args(schema: &serde_json::Value) -> bool {
    // Check if there's a "required" array with any entries
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .is_some_and(|arr| !arr.is_empty())
}

#[allow(clippy::future_not_send)]
#[allow(clippy::missing_panics_doc)]
pub async fn start_agent_server(
    config: &AgentConfig,
    trust: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::mcp_spawner::McpProcessManager;
    use arkavo_crypto::AgentKeypair;
    use arkavo_gossip::GossipConfig;
    use arkavo_protocol::config::ServerConfig;
    use arkavo_server::A2aServer;
    use arkavo_server::{
        LearningBus, start_advisor_broadcast_loop, start_anti_entropy_loop,
        start_lesson_propagation_loop,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    // First, before any identity or process state is created: a listen
    // address that cannot be understood must leave nothing behind.
    let listen_addr = listen::parse_listen(&config.listen)?;

    // Load or create persisted device keypair (Phase 1 identity anchor)
    use arkavo_device_identity::keypair as device_keypair_store;
    let device_keypair = {
        let bytes = match device_keypair_store::get_keypair()? {
            Some(bytes) => bytes,
            None => {
                let new_kp = arkavo_crypto::AgentKeypair::generate();
                let bytes = new_kp.to_bytes();
                device_keypair_store::store_keypair(&bytes)?;
                bytes
            }
        };
        Arc::new(
            arkavo_crypto::AgentKeypair::from_bytes(&bytes).expect("Invalid device keypair bytes"),
        )
    };
    let device_did = device_keypair.public_key().to_did_key();

    // Create AgentCredential for TDF encryption/decryption
    let mut identity_attributes = HashMap::new();
    identity_attributes.insert("agent.id".to_string(), config.name.clone());
    let agent_identity = Arc::new(
        AgentCredential::new(config.name.clone(), identity_attributes)
            .map_err(|e| format!("Failed to create AgentCredential: {e}"))?,
    );

    // Encode public key as base64 for mDNS broadcast
    let public_key_b64 = BASE64_STANDARD.encode(agent_identity.public_key.as_bytes());

    let debug_mode = std::env::var("ARKAVO_DEBUG").is_ok();
    if debug_mode {
        println!(
            "[Identity] Created AgentIdentity for agent '{}' with public key: {}...",
            config.name,
            &public_key_b64[..20.min(public_key_b64.len())]
        );
    }

    // Create process manager for MCP servers
    let process_manager = McpProcessManager::new();

    // Create shutdown flag for mDNS thread
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    // Use absolute path for task store to avoid issues with directory changes
    let task_store_path = std::env::current_dir()?
        .join(".arkavo")
        .join("arkavo_tasks.db");

    let server_config = ServerConfig {
        enabled: true,
        bind_address: listen_addr.ip().to_string(),
        port: listen_addr.port(),
        max_connections: 100,
        idle_timeout_seconds: 300,
        rate_limit: listen::rpc_rate_limit(),
        task_store_path: Some(task_store_path.to_string_lossy().to_string()),
        metrics_enabled: true,
    };

    let server = A2aServer::new(server_config);

    // Set the public key for TDF encryption (used in agent.capabilities.get RPC)
    server.set_public_key(public_key_b64.clone()).await;

    // Create pain signal channel before LearningBus (lock-free hot path)
    let (pain_tx, pain_rx) = tokio::sync::mpsc::channel::<arkavo_server::PainSignal>(128);

    // Initialize LearningBus FIRST (before set_agent_metadata which initializes router)
    let learning_bus = {
        let keypair = Arc::new(AgentKeypair::generate());
        let gossip_config = GossipConfig::default();
        let swarm_id = config.swarm.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "default-swarm".to_string())
        });
        let mut bus = LearningBus::new(config.name.clone(), swarm_id, keypair, gossip_config);
        bus.set_pain_sender(pain_tx);
        Arc::new(bus)
    };
    // Initialize persistent learning store (SQLite) for lessons
    {
        let db_path = std::path::PathBuf::from(".arkavo/learning/lessons.db");
        learning_bus.init_persistence(&db_path).await;
    }

    server.set_learning_bus(learning_bus.clone()).await;

    // Register learning pipeline health reporter for self-check
    arkavo_server::LearningPipelineReporter::register(learning_bus.clone()).await;

    // Set agent metadata (this initializes the router which will be set on learning bus)
    server
        .set_agent_metadata(
            config.name.clone(),
            config.purpose.clone(),
            config.model.clone(),
            config.mode.clone(),
            Some(device_did.clone()),
        )
        .await;

    server.set_agent_role(config.role_id.clone()).await;
    server
        .set_agent_description(config.description.clone())
        .await;

    // Set API keys in the server
    server.set_api_keys(config.api_keys.clone()).await;

    // The router is on the bus by now, and its feasible arm set is fixed when it
    // is built (`set_api_keys` above only records metadata), so whether cloud
    // augmentation is available at all is already answerable. Ask once, here,
    // while nothing else is running: the conductor issues routing calls this
    // command does not, and it has no second chance to reach the operator.
    // Cloned out of the guard first — the prompt below awaits an answer and the
    // read lock must not be held while it waits.
    let startup_router = learning_bus.router().read().await.clone();
    if let Some(router) = startup_router {
        confirm_cloud_startup(&router).await;
    }

    // Initialize MCP connections from agent config only.
    // Built-in tools are not registered - agents use only their configured MCP servers.
    // This enables small models (ministral-3b) to work with focused tool sets.
    let mcp_registry = server.mcp_registry();

    let debug_mode = std::env::var("ARKAVO_DEBUG").is_ok();
    let quiet = config.quiet;

    for mcp_config in &config.mcp_servers {
        if debug_mode {
            println!(
                "[MCP] Initializing server: name={} command={:?} args={:?}",
                mcp_config.name, mcp_config.command, mcp_config.args
            );
        }

        // Create appropriate MCP connection based on config
        if let Some(command) = &mcp_config.command {
            // Create MCP client using the existing sync approach
            use crate::mcp_client::McpClient;
            use crate::mcp_integration::McpConnection;

            match McpClient::new_with_command(command, &mcp_config.args) {
                Ok(mut client) => {
                    // Set server name for poll notifications
                    client.set_server_name(mcp_config.name.clone());

                    // Register the spawned process with the process manager for cleanup
                    let pid = client.pid().unwrap_or(0);
                    if pid > 0 {
                        process_manager.register_process(mcp_config.name.clone(), pid);
                    }

                    // Get tools and set up polling for "read-*" and "get-*" tools
                    let tools = client.list_tools().unwrap_or_default();
                    let tool_count = tools.len();

                    // Wrap in Arc for polling support
                    let client = std::sync::Arc::new(client);

                    // Auto-register pollable tools (read-* pattern with no required args)
                    let poll_count = {
                        use arkavo_mcp_runtime::polling::PollableEndpoint;
                        let mut count = 0;
                        for tool in &tools {
                            // Only poll read-* tools that have no required arguments
                            if tool.name.starts_with("read-")
                                && !has_required_args(&tool.input_schema)
                            {
                                client.register_pollable(PollableEndpoint {
                                    method: "tools/call".to_string(),
                                    params: Some(serde_json::json!({
                                        "name": tool.name,
                                        "arguments": {}
                                    })),
                                });
                                count += 1;
                                if debug_mode {
                                    println!("[MCP] Registered pollable tool: {}", tool.name);
                                }
                            }
                        }
                        count
                    };

                    // Set up notification forwarding BEFORE starting polling
                    // (broadcast receivers only get messages sent after they subscribe)
                    let mut notif_rx = client.subscribe_notifications();
                    let registry_clone = mcp_registry.clone();
                    let server_name_for_notif = mcp_config.name.clone();
                    tokio::spawn(async move {
                        use arkavo_protocol::mcp_registry::McpNotification;
                        while let Ok(notification) = notif_rx.recv().await {
                            registry_clone.emit_notification(McpNotification {
                                server: server_name_for_notif.clone(),
                                method: notification.method,
                                params: notification.params,
                            });
                        }
                    });

                    // Start polling AFTER subscribing to notifications
                    if poll_count > 0 {
                        use arkavo_mcp_runtime::polling::PollConfig;
                        client.start_polling(PollConfig::default());
                        println!(
                            "[MCP] Started polling for {} tool(s) on server {}",
                            poll_count, mcp_config.name
                        );
                    }

                    // Unwrap Arc for McpConnection (client is Clone, all fields are Arc-wrapped)
                    let connection = McpConnection::External(
                        std::sync::Arc::try_unwrap(client).unwrap_or_else(|arc| (*arc).clone()),
                    );
                    let wrapped = McpConnectionWrapper::new(connection);
                    mcp_registry
                        .register(mcp_config.name.clone(), Box::new(wrapped))
                        .await;

                    if debug_mode {
                        println!(
                            "[MCP] Server started: name={} command={} pid={} tools={}",
                            mcp_config.name, command, pid, tool_count
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[MCP] Server FAILED: name={} command={} error=\"{}\"",
                        mcp_config.name, command, e
                    );
                }
            }
        } else if let Some(url) = &mcp_config.url {
            // Create external MCP connection (HTTP streamable or subprocess)
            use crate::mcp_integration::McpConnection;
            match McpConnection::new_external(Some(url.clone())) {
                Ok(connection) => {
                    let tool_count = connection.list_tools().map(|t| t.len()).unwrap_or(0);
                    let wrapped = McpConnectionWrapper::new(connection);
                    mcp_registry
                        .register(mcp_config.name.clone(), Box::new(wrapped))
                        .await;

                    if !quiet || debug_mode {
                        println!(
                            "[MCP] External server connected: name={} url={} tools={}",
                            mcp_config.name, url, tool_count
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[MCP] External server FAILED: name={} url={} error=\"{}\"",
                        mcp_config.name, url, e
                    );
                }
            }
        }
    }

    // Log summary of all registered MCP servers
    if debug_mode {
        match mcp_registry.list_all_tools().await {
            Ok(tools) => {
                println!(
                    "[MCP] Total tools registered: {} from {} server(s)",
                    tools.len(),
                    config.mcp_servers.len()
                );
                for tool in &tools {
                    println!("[MCP]   - {}: {}", tool.name, tool.description);
                }
            }
            Err(e) => {
                eprintln!("[MCP] Failed to list tools: {e}");
            }
        }
    }

    // Router is initialized by A2aServer when set_agent_metadata() is called with an empty model.
    // The Conductor is directly integrated in A2aRpcImpl for task execution.
    if debug_mode {
        println!("HRM Conductor integrated for A2A task execution");
    }

    let (handle, bound_addr) = server.start_with_addr().await?;

    // Shown even in a quiet run: whoever started the agent has to learn that
    // it is reachable from the network whether or not they asked for output.
    // This is every start that neither `--bind` nor the kit put on loopback,
    // a `--trust` start included: the QR code it shows is for a device that
    // has to reach the address.
    if let Some(warning) = listen::exposure_warning(bound_addr) {
        eprintln!("{warning}");
    }

    // One address for everything handed to clients: the agent card, the
    // authorization URL and, below, the mDNS record.
    let endpoint = advertise::endpoint_url(advertise::advertised_addr(bound_addr, local_ip));
    server.set_advertised_endpoint(endpoint.clone()).await;

    // Start orchestrator loop for any agent with a purpose
    if !config.purpose.is_empty() {
        server.start_orchestrator_loop().await;
    }

    // Generate and display QR code for registration. Shown in verbose runs, or on demand
    // via `--trust` (which surfaces only the QR, without the rest of the verbose output).
    if !quiet || trust {
        use arkavo_device_identity::get_or_create_device_id;
        use arkavo_registration::{AgentDescriptor, qr::display_authorization_qr};

        // Get or create device ID (needed for system initialization)
        let _device_id =
            get_or_create_device_id().map_err(|e| format!("Failed to get device ID: {e}"))?;

        // Reuse persisted device keypair loaded at startup (Phase 1 identity)
        let public_key = device_keypair.public_key();

        // Extract folder name (last part of agent name) for display
        let folder_id = config
            .name
            .rsplit('-')
            .next()
            .unwrap_or("unknown")
            .to_string();

        // Create agent descriptor with DID:key and default entitlements
        let mdns_service = if config.mdns_enabled {
            Some(format!("{}._a2a._tcp.local.", config.name))
        } else {
            None
        };

        let descriptor = AgentDescriptor::new(public_key, endpoint, mdns_service, folder_id)
            .with_name(&config.name)
            .with_entitlements(vec![
                "agent.capability.chat".to_string(),
                "agent.capability.tools".to_string(),
            ]);

        // Display authorization QR code with DID:key
        println!("\n{}", "=".repeat(60));
        println!("Agent Authorization QR Code");
        println!("{}", "=".repeat(60));
        if let Err(e) = display_authorization_qr(&descriptor) {
            eprintln!("Warning: Failed to display QR code: {e}");
        }
        println!("{}", "=".repeat(60));
    }

    // Channel for peer discovery events (bridges sync mDNS thread to async LearningBus)
    // Tuple: (peer_id, is_add, address)
    let (peer_tx, mut peer_rx) = tokio::sync::mpsc::channel::<(String, bool, Option<String>)>(100);

    // Start mDNS broadcasting if enabled
    let mdns_thread_handle = if config.mdns_enabled {
        let config_clone = config.clone();
        let shutdown_flag_clone = shutdown_flag.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let peer_tx_clone = peer_tx.clone();
        let public_key_clone = public_key_b64.clone();

        // Use std::thread since zeroconf is not Send
        let handle = std::thread::spawn(move || {
            if let Err(e) = broadcast_agent_mdns_sync(
                &config_clone,
                bound_addr,
                shutdown_flag_clone,
                Some(tx),
                peer_tx_clone,
                Some(public_key_clone),
            ) {
                eprintln!("mDNS broadcast error: {e}");
            }
        });

        // Wait for mDNS to signal it's ready (with timeout)
        let _ = rx.recv_timeout(std::time::Duration::from_secs(2));

        Some(handle)
    } else {
        None
    };

    // Start gossip learning background tasks when A2A is enabled
    // Peers are added dynamically via mDNS discovery
    let mut gossip_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    if config.a2a_enabled {
        // Start anti-entropy background task (5s interval)
        let learning_bus_ae = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            start_anti_entropy_loop(learning_bus_ae, Duration::from_secs(5)).await;
        }));

        // Start lesson propagation background task (15s interval)
        let learning_bus_lp = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            start_lesson_propagation_loop(learning_bus_lp, Duration::from_secs(15)).await;
        }));

        // Start advisor adjustment broadcast loop (60s interval)
        let learning_bus_adv = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            start_advisor_broadcast_loop(learning_bus_adv, Duration::from_mins(1)).await;
        }));

        // Start lesson application loop (processes approved lessons, adds to policy cache)
        let learning_bus_apply = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            if let Some(rx) = learning_bus_apply.subscribe_lesson_approvals().await {
                arkavo_server::start_lesson_application_loop(learning_bus_apply, rx).await;
            } else {
                tracing::warn!("Lesson application loop: could not subscribe to approvals");
            }
        }));

        // Start event processing loop (converts observations to episodes to lessons)
        let learning_bus_events = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            if let Some(rx) = learning_bus_events.take_event_receiver().await {
                arkavo_server::start_event_processing_loop(learning_bus_events, rx).await;
            } else {
                tracing::warn!("Event processing loop: event receiver already taken");
            }
        }));

        // Start peer discovery handler (receives from mDNS thread, updates LearningBus)
        let learning_bus_peers = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            use arkavo_protocol::http::HttpTransport;
            use arkavo_protocol::transport::{
                A2aEndpoint, A2aRequest, A2aTransport, TransportConfig,
            };

            while let Some((peer_id, is_add, address)) = peer_rx.recv().await {
                if is_add {
                    learning_bus_peers
                        .add_peer_discovered(peer_id.clone(), address.clone())
                        .await;

                    // Initiate key exchange with the peer
                    if let Some(addr) = address {
                        let our_public_key = learning_bus_peers.keypair().public_key().to_base64();
                        let our_agent_id = learning_bus_peers.agent_id().to_string();

                        let request = A2aRequest::new(
                            "agent/exchangeKeys",
                            serde_json::json!({
                                "peer_id": our_agent_id,
                                "public_key": our_public_key
                            }),
                        );

                        let mut config = TransportConfig::default();
                        config.tls_config.require_tls = false;
                        let transport = match HttpTransport::new(config) {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to create transport for key exchange: {}",
                                    e
                                );
                                continue;
                            }
                        };

                        let endpoint = A2aEndpoint {
                            url: addr.clone(),
                            agent_id: peer_id.clone(),
                            public_key: None,
                        };

                        if let Err(e) = transport.connect(&endpoint).await {
                            tracing::warn!(
                                "Failed to connect for key exchange with {}: {}",
                                peer_id,
                                e
                            );
                            continue;
                        }

                        match transport.send_request(request).await {
                            Ok(response) => {
                                use arkavo_protocol::transport::A2aResponse;
                                // Parse the response to get their public key
                                if let A2aResponse::Success { result, .. } = response
                                    && let Some(their_key_b64) = result.as_str()
                                {
                                    match arkavo_crypto::AgentPublicKey::from_base64(their_key_b64)
                                    {
                                        Ok(their_key) => {
                                            learning_bus_peers
                                                .register_peer_key(peer_id.clone(), their_key)
                                                .await;
                                            tracing::info!(
                                                "Key exchange completed with peer: {}",
                                                peer_id
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Invalid public key from {}: {}",
                                                peer_id,
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Key exchange failed with {}: {}", peer_id, e);
                            }
                        }
                    }
                } else {
                    learning_bus_peers.remove_peer(&peer_id).await;
                }
            }
        }));

        // Start gossip message transport (sends outgoing messages to peers via A2A)
        // Prefers persistent WebSocket connections, falls back to HTTP per-message.
        let learning_bus_transport = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            use arkavo_protocol::http::HttpTransport;
            use arkavo_protocol::transport::{
                A2aEndpoint, A2aRequest, A2aTransport, A2aTransportRef, TransportConfig,
            };
            use arkavo_protocol::websocket::WebSocketTransport;
            use std::collections::HashMap;
            use std::sync::Arc;

            let mut connections: HashMap<String, A2aTransportRef> = HashMap::new();

            let mut rx = learning_bus_transport.subscribe_gossip_out();
            loop {
                match rx.recv().await {
                    Ok((peer_id, message)) => {
                        if let Some(addr) = learning_bus_transport.get_peer_address(&peer_id).await
                        {
                            let params = serde_json::json!({ "message": message });
                            let request = A2aRequest::new("gossip/message", params);

                            // Reconnect if connection dropped or missing
                            let needs_reconnect =
                                connections.get(&peer_id).is_none_or(|c| !c.is_connected());

                            if needs_reconnect {
                                connections.remove(&peer_id);
                                let mut config = TransportConfig::default();
                                config.tls_config.require_tls = false;

                                // Try WebSocket first (persistent), fall back to HTTP
                                let ws_url = addr.replace("http://", "ws://");
                                let ws = WebSocketTransport::new(config.clone());
                                let endpoint = A2aEndpoint {
                                    url: ws_url,
                                    agent_id: peer_id.clone(),
                                    public_key: None,
                                };

                                if ws.connect(&endpoint).await.is_ok() {
                                    tracing::info!("Gossip: WebSocket connected to {}", peer_id);
                                    connections.insert(peer_id.clone(), Arc::new(ws));
                                } else if let Ok(http) = HttpTransport::new(config) {
                                    let endpoint = A2aEndpoint {
                                        url: addr.clone(),
                                        agent_id: peer_id.clone(),
                                        public_key: None,
                                    };
                                    if http.connect(&endpoint).await.is_ok() {
                                        tracing::info!("Gossip: HTTP fallback to {}", peer_id);
                                        connections.insert(peer_id.clone(), Arc::new(http));
                                    } else {
                                        tracing::warn!("Gossip: failed to connect to {}", peer_id);
                                        continue;
                                    }
                                } else {
                                    tracing::warn!("Gossip: transport error for {}", peer_id);
                                    continue;
                                }
                            }

                            if let Some(conn) = connections.get(&peer_id) {
                                match conn.send_request(request).await {
                                    Ok(_) => {
                                        tracing::debug!("Gossip sent to {}", peer_id);
                                    }
                                    Err(e) => {
                                        tracing::warn!("Gossip send failed to {}: {}", peer_id, e);
                                        connections.remove(&peer_id);
                                    }
                                }
                            }
                        } else {
                            tracing::debug!("No address for peer {}, skipping gossip", peer_id);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Gossip transport lagged {} messages", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));

        // Start AutoLearner self-healing loop
        match arkavo_server::AutoLearnBridge::new(
            config.name.clone(),
            learning_bus.clone(),
            pain_rx,
        ) {
            Ok(bridge) => {
                gossip_handles.push(bridge.handle);
                tracing::info!("AutoLearn self-healing loop started");
            }
            Err(e) => {
                tracing::warn!("AutoLearn unavailable: {e}");
            }
        }

        if !quiet {
            println!("Gossip learning: background tasks started");
        }
    }

    // Start push-based notification handler (always on by default)
    let notification_handle = server
        .start_notification_handler(config.purpose.clone())
        .await;

    if notification_handle.is_some() && !quiet {
        println!("Notification handler: listening for MCP push events");
    }

    if !quiet {
        // The bound address carries the port the OS assigned when the kit asked for port 0.
        println!("Ready at {bound_addr}");
    }

    // Keep the server running
    tokio::signal::ctrl_c().await?;

    // Stop notification handler if running
    if let Some(handle) = notification_handle {
        handle.abort();
    }

    // Stop gossip background tasks
    for handle in gossip_handles {
        handle.abort();
    }

    println!("Shutting down agent server...");

    // Signal the mDNS thread to stop
    shutdown_flag.store(true, Ordering::Relaxed);

    // Stop the A2A server
    handle.stop()?;

    // Release GPU resources before exit to ensure Metal residency sets are cleaned up
    server.cleanup_gpu_resources().await;

    // Shutdown all MCP processes
    process_manager.shutdown_all()?;

    // Wait for mDNS thread to finish (with timeout)
    if let Some(handle) = mdns_thread_handle {
        println!("Waiting for mDNS thread to stop...");
        // Give it 2 seconds to stop gracefully
        let timeout = std::time::Duration::from_secs(2);
        let start = std::time::Instant::now();

        while !handle.is_finished() && start.elapsed() < timeout {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if !handle.is_finished() {
            eprintln!("Warning: mDNS thread did not stop within timeout");
            // Thread will be forcefully terminated when process exits
        }
    }

    println!("Agent server stopped.");

    // Use _exit() to terminate without running C++ static destructors.
    // std::process::exit() calls libc exit() which triggers __cxa_finalize,
    // and the ggml Metal device static destructor asserts all GPU resource
    // sets are freed. Aborted tokio tasks may still hold Arc<LlamaModel>
    // references, preventing full cleanup before the static destructor runs.
    #[cfg(unix)]
    unsafe {
        libc::_exit(0);
    }
    #[cfg(not(unix))]
    std::process::exit(0);
}

/// The address of this machine that other machines can reach, for an
/// endpoint bound to every interface.
///
/// Uses multiple strategies to determine it:
/// 1. Try connecting to a public DNS server (determines routing interface)
/// 2. Try connecting to a common private gateway (offline LANs)
/// 3. Final fallback to 127.0.0.1, which only local clients can use
///
/// This handles offline environments, strict firewalls, and IPv6-only networks.
fn local_ip() -> std::net::IpAddr {
    use std::net::{IpAddr, Ipv4Addr, UdpSocket};

    let targets = [
        // Public DNS servers: work in most online environments.
        ("8.8.8.8", 80),        // Google DNS
        ("1.1.1.1", 80),        // Cloudflare DNS
        ("208.67.222.222", 80), // OpenDNS
        // Private gateways: work in offline LAN environments.
        ("192.168.1.1", 53), // Common router address
        ("10.0.0.1", 53),    // Common corporate router
        ("172.16.0.1", 53),  // Common large network router
    ];

    for (target, port) in targets {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0")
            && socket.connect((target, port)).is_ok()
            && let Ok(local_addr) = socket.local_addr()
        {
            let ip = local_addr.ip();
            if !ip.is_loopback() && ip.is_ipv4() {
                return ip;
            }
        }
    }

    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

fn broadcast_agent_mdns_sync(
    #[allow(unused_variables)] config: &AgentConfig,
    #[allow(unused_variables)] bound_addr: std::net::SocketAddr,
    #[allow(unused_variables)] shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[allow(unused_variables)] ready_signal: Option<std::sync::mpsc::Sender<()>>,
    #[allow(unused_variables)] peer_tx: tokio::sync::mpsc::Sender<(String, bool, Option<String>)>,
    #[allow(unused_variables)] public_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mdns")]
    {
        use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
        use std::thread;
        use std::time::Duration;

        let port = bound_addr.port();
        let service_ip =
            advertise::advertised_addr(bound_addr, || std::net::IpAddr::V4(get_service_ip())).ip();

        // Create mDNS daemon
        let mdns = ServiceDaemon::new()?;

        // mdns-sd leaves loopback interfaces out unless asked, and announces
        // an address only on the interface whose network holds it. Without
        // this a loopback address is announced nowhere, not even to this
        // machine; with it the record still never leaves the machine. An
        // agent on a network address needs it too: this daemon also browses,
        // and an agent bound to loopback is announced on loopback only.
        mdns.enable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])?;

        // Start browsing for other agents
        let receiver = mdns.browse("_a2a._tcp.local.")?;

        // Clone shutdown flag for discovery thread
        let shutdown_flag_discovery = shutdown_flag.clone();

        // Clone our agent name to filter out self-discovery
        let our_agent_name = config.name.clone();

        // Spawn a thread to handle discovered services
        let discovery_thread = thread::spawn(move || {
            use mdns_sd::ServiceEvent;
            use std::collections::HashSet;
            println!("Starting discovery of other agents...");

            // Track discovered peers to avoid duplicates
            let mut discovered_peers: HashSet<String> = HashSet::new();

            loop {
                match receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(event) => match event {
                        ServiceEvent::ServiceResolved(info) => {
                            // Filter out self-discovery
                            if let Some(agent_id) = info.get_property_val_str("agent_id") {
                                if agent_id == our_agent_name {
                                    // Skip - this is our own service
                                    continue;
                                }

                                println!("Agent discovered: {}", info.get_fullname());
                                println!("  - Agent ID: {agent_id}");

                                if let Some(purpose) = info.get_property_val_str("purpose") {
                                    println!("  - Purpose: {purpose}");
                                }
                                // Get peer address
                                let peer_addr = info
                                    .get_addresses()
                                    .iter()
                                    .next()
                                    .map(|addr| format!("http://{}:{}", addr, info.get_port()));

                                if let Some(ref addr) = peer_addr {
                                    println!("  - Address: {addr}");
                                }

                                // Notify LearningBus of new peer (only once per agent_id)
                                if discovered_peers.insert(agent_id.to_string()) {
                                    let _ = peer_tx.blocking_send((
                                        agent_id.to_string(),
                                        true,
                                        peer_addr,
                                    ));
                                }
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            println!("Agent disconnected: {fullname}");
                            // Extract agent_id from fullname (e.g., "rover-beta._a2a._tcp.local.")
                            if let Some(agent_id) = fullname.split("._a2a._tcp.local.").next()
                                && discovered_peers.remove(agent_id)
                            {
                                let _ = peer_tx.blocking_send((agent_id.to_string(), false, None));
                            }
                        }
                        _ => {}
                    },
                    Err(_) => {
                        // Timeout - check if we should shutdown
                        if shutdown_flag_discovery.load(std::sync::atomic::Ordering::Relaxed) {
                            println!("Discovery thread shutting down...");
                            break;
                        }
                    }
                }
            }
        });

        // Capability tags are coarse routing labels; they are derived from
        // the purpose but do not carry any of its text.
        let capabilities = get_agent_capabilities(&config.name, &config.purpose);
        let properties =
            advertise::txt_properties(config, service_ip, public_key.as_deref(), &capabilities);

        // Create service info
        let service_type = "_a2a._tcp.local.";
        let instance_name = config.name.clone();
        let host_name = format!("{}.local.", config.name);

        let service_info = ServiceInfo::new(
            service_type,
            &instance_name,
            &host_name,
            service_ip,
            port,
            properties,
        )?;

        // Register the service
        mdns.register(service_info)?;

        // Signal that mDNS is ready
        if let Some(tx) = ready_signal {
            let _ = tx.send(());
        }

        // Keep the service alive until shutdown
        use std::sync::atomic::Ordering;
        while !shutdown_flag.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
            // Keep reference to prevent dropping
            let _ = &mdns;
        }

        println!("mDNS service shutting down...");

        // Wait for discovery thread to finish
        let _ = discovery_thread.join();

        // Service will be unregistered when mdns goes out of scope
    }

    #[cfg(not(feature = "mdns"))]
    {
        // Signal ready immediately when mDNS is not compiled
        if let Some(tx) = ready_signal {
            let _ = tx.send(());
        }

        // Keep the thread alive until shutdown is signaled
        use std::sync::atomic::Ordering;
        while !shutdown_flag.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    Ok(())
}

// Wrapper to implement McpClient trait from arkavo-mcp for arkavo-cli's McpConnection
struct McpConnectionWrapper {
    inner: crate::mcp_integration::McpConnection,
}

impl McpConnectionWrapper {
    fn new(connection: crate::mcp_integration::McpConnection) -> Self {
        Self { inner: connection }
    }
}

impl arkavo_mcp::McpClient for McpConnectionWrapper {
    fn list_tools(
        &self,
    ) -> Result<Vec<arkavo_mcp::McpTool>, Box<dyn std::error::Error + Send + Sync>> {
        // Convert from cli Tool to McpTool
        let cli_tools = self.inner.list_tools().map_err(|e| {
            Box::new(std::io::Error::other(e.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;
        let protocol_tools = cli_tools
            .into_iter()
            .map(|t| arkavo_mcp::McpTool {
                name: t.name,
                description: t.description,
                input_schema: Some(t.input_schema),
            })
            .collect();
        Ok(protocol_tools)
    }

    fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        llm_provider: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.inner
            .call_tool(tool_name, arguments, llm_provider)
            .map_err(|e| {
                Box::new(std::io::Error::other(e.to_string()))
                    as Box<dyn std::error::Error + Send + Sync>
            })
    }
}

/// Determine agent capabilities based on name and purpose
#[cfg(feature = "mdns")]
fn get_agent_capabilities(name: &str, purpose: &str) -> Vec<String> {
    let mut capabilities = Vec::new();

    // Extract capabilities from agent name and purpose
    let combined = format!("{} {}", name.to_lowercase(), purpose.to_lowercase());

    // Domain-specific capabilities
    if combined.contains("orchestrat") {
        capabilities.push("orchestration".to_string());
        capabilities.push("task_decomposition".to_string());
        capabilities.push("agent_coordination".to_string());
    }
    if combined.contains("security") {
        capabilities.push("security_analysis".to_string());
        capabilities.push("vulnerability_detection".to_string());
    }
    if combined.contains("code") || combined.contains("review") {
        capabilities.push("code_review".to_string());
        capabilities.push("pattern_analysis".to_string());
    }
    if combined.contains("database") || combined.contains("sql") {
        capabilities.push("database_optimization".to_string());
        capabilities.push("schema_design".to_string());
    }
    if combined.contains("test") {
        capabilities.push("test_generation".to_string());
        capabilities.push("coverage_analysis".to_string());
    }
    if combined.contains("doc") {
        capabilities.push("documentation_generation".to_string());
        capabilities.push("api_documentation".to_string());
    }
    if combined.contains("performance") || combined.contains("profil") {
        capabilities.push("performance_analysis".to_string());
        capabilities.push("optimization".to_string());
    }
    if combined.contains("devops") || combined.contains("deploy") {
        capabilities.push("ci_cd".to_string());
        capabilities.push("deployment_strategies".to_string());
    }
    if combined.contains("frontend") || combined.contains("ui") || combined.contains("ux") {
        capabilities.push("ui_ux_analysis".to_string());
        capabilities.push("accessibility".to_string());
    }
    if combined.contains("architect") || combined.contains("design") {
        capabilities.push("system_design".to_string());
        capabilities.push("scalability_patterns".to_string());
    }
    if combined.contains("data") || combined.contains("science") || combined.contains("ml") {
        capabilities.push("data_analysis".to_string());
        capabilities.push("ml_modeling".to_string());
    }

    capabilities
}

/// Whether startup should ask the operator to authorize cloud inference.
///
/// The harness requires local inference, so the resolved execution arm is
/// always local and a cloud arm can only ever augment it. Under the default
/// `AskBeforeCloud` policy the router refuses that augmentation until someone
/// says yes, and it has no channel of its own to ask. Asking once here turns
/// the refusal into a decision. An install with no cloud credentials has
/// nothing to approve, and a non-interactive run keeps the error path — an
/// unattended agent must not spend against a prompt nobody answered.
fn cloud_startup_confirmation_needed(
    policy: arkavo_budget::CloudPolicy,
    cloud_available: bool,
    interactive: bool,
    already_approved: bool,
) -> bool {
    matches!(policy, arkavo_budget::CloudPolicy::AskBeforeCloud)
        && cloud_available
        && interactive
        && !already_approved
}

/// Prompt once, before the server accepts work, and record the answer for the
/// host's own routing calls on a yes.
///
/// The approval is keyed to the host, not to a session: `arkavo agent` issues
/// no routing calls itself — the conductor does, many per task, starting with
/// intent decomposition — and those carry no session id. It therefore never
/// authorizes a chat session, which always names itself when it routes, so a
/// remote client of this agent is still asked its own question.
async fn confirm_cloud_startup(router: &arkavo_router::Router) {
    use crate::cloud_consent::TtyCloudConsent;
    use arkavo_router::{CloudConsentPrompt, CloudConsentRequest};

    if !cloud_startup_confirmation_needed(
        router.cloud_policy(),
        router.cloud_augmentation_available(),
        TtyCloudConsent::is_interactive(),
        router.cloud_approved(None),
    ) {
        return;
    }

    if TtyCloudConsent::new()
        .ask(CloudConsentRequest::Session)
        .await
    {
        router.approve_cloud_for_host();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_budget::CloudPolicy;
    use arkavo_test_macros::spec;

    #[test]
    #[spec("ASTRA-004")]
    fn an_interactive_startup_with_cloud_arms_asks_once() {
        assert!(cloud_startup_confirmation_needed(
            CloudPolicy::AskBeforeCloud,
            true,
            true,
            false
        ));
    }

    /// A standing approval on the router answers the question, so a resumed or
    /// re-entered startup path does not ask again.
    #[test]
    #[spec("ASTRA-004")]
    fn a_standing_approval_suppresses_the_prompt() {
        assert!(!cloud_startup_confirmation_needed(
            CloudPolicy::AskBeforeCloud,
            true,
            true,
            true
        ));
    }

    /// The harness always resolves to a local arm, so the question is only
    /// worth asking when a cloud arm is configured to augment it.
    #[test]
    #[spec("ASTRA-004")]
    fn an_install_with_no_cloud_arm_is_never_prompted() {
        assert!(!cloud_startup_confirmation_needed(
            CloudPolicy::AskBeforeCloud,
            false,
            true,
            false
        ));
    }

    /// Regression: an unattended run must reach the router's policy error
    /// rather than block on a console nobody is watching.
    #[test]
    #[spec("ASTRA-004")]
    fn non_tty_keeps_the_error_path() {
        assert!(!cloud_startup_confirmation_needed(
            CloudPolicy::AskBeforeCloud,
            true,
            false,
            false
        ));
    }

    #[test]
    #[spec("ASTRA-004")]
    fn other_policies_do_not_prompt() {
        // LocalOnly refuses cloud outright and CloudWithinCap already authorizes
        // it; neither is a question for the operator.
        for policy in [CloudPolicy::LocalOnly, CloudPolicy::CloudWithinCap] {
            assert!(!cloud_startup_confirmation_needed(
                policy, true, true, false
            ));
        }
    }

    // An unrecognized option must error, not silently boot an agent (regression: the bare
    // `arkavo <flag>` route dispatches here, and unknown dash args were previously ignored).
    #[test]
    fn unknown_option_errors_instead_of_running() {
        assert!(execute(&["--bogus".to_string()]).is_err());
        assert!(execute(&["--trsut".to_string()]).is_err()); // typo of --trust
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    /// The run options a command line parses to.
    fn run_options(values: &[&str]) -> RunOptions {
        match parse_args(&args(values)).unwrap() {
            Invocation::Run(options) => options,
            other => panic!("{values:?} is not a run: {other:?}"),
        }
    }

    /// `--trust` shows the QR code and decides nothing else; in particular
    /// it leaves `bind` unset, so the agent listens where it would without
    /// the flag.
    #[test]
    fn trust_sets_only_the_qr_code() {
        assert_eq!(
            run_options(&["--trust"]),
            RunOptions {
                trust: true,
                ..RunOptions::default()
            }
        );
        assert_eq!(
            run_options(&["run", "--trust", "-p", "8343"]),
            RunOptions {
                trust: true,
                port: Some(8343),
                ..RunOptions::default()
            }
        );
    }

    #[test]
    fn bind_is_read_with_and_without_a_port() {
        for (value, bind) in [
            ("127.0.0.1", ("127.0.0.1", None)),
            ("127.0.0.1:8342", ("127.0.0.1", Some(8342))),
            ("[::1]:8342", ("::1", Some(8342))),
            ("0.0.0.0", ("0.0.0.0", None)),
        ] {
            let options = run_options(&["--bind", value, "-p", "9000", "--trust"]);
            let expected = listen::BindAddress {
                ip: bind.0.parse().unwrap(),
                port: bind.1,
            };
            assert_eq!(options.bind, Some(expected), "{value}");
            assert_eq!(options.port, Some(9000), "{value}");
            assert!(options.trust, "{value}");
        }
    }

    #[test]
    fn an_unparseable_bind_is_a_startup_error() {
        for bad in ["localhost", "localhost:8342", "8342", "not an address"] {
            let err = parse_args(&args(&["--bind", bad]))
                .expect_err(bad)
                .to_string();
            assert!(err.contains("Invalid bind address"), "{bad:?}: {err}");
        }
        let err = parse_args(&args(&["--bind"])).unwrap_err().to_string();
        assert!(err.contains("--bind"), "{err}");
        let err = parse_args(&args(&["--bind", "--trust"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("--bind"), "{err}");
    }

    /// The help must match the loopback default and keep QR authorization
    /// separate from the address selected by `--bind`.
    #[test]
    fn help_describes_the_network_default_and_the_bind_option() {
        let (options, network) = USAGE
            .split_once("NETWORK:")
            .expect("the help has a NETWORK section");
        let network = network.split("EXAMPLES:").next().unwrap();

        let trust = options
            .split_once("--trust")
            .expect("--trust is listed under OPTIONS")
            .1
            .lines()
            .next()
            .unwrap();
        assert!(trust.contains("QR code"), "{trust}");
        assert!(!options.contains("loopback"), "{options}");

        let bind = options
            .split_once("--bind <ADDRESS>")
            .expect("--bind is listed under OPTIONS")
            .1;
        assert!(bind.contains("127.0.0.1"), "{bind}");
        assert!(bind.contains("runtime.listen"), "{bind}");
        assert!(bind.contains("-p"), "{bind}");

        assert!(network.contains("listens on loopback"), "{network}");
        assert!(network.contains("mDNS"), "{network}");
        assert!(network.contains("not authenticated"), "{network}");
        assert!(network.contains("notice"), "{network}");
        assert!(network.contains("--bind 127.0.0.1"), "{network}");
        assert!(network.contains("runtime.listen"), "{network}");
        assert!(!network.contains("--trust"), "{network}");
        assert!(
            !network.contains("listens on 127.0.0.1 unless"),
            "{network}"
        );
    }

    /// A single-role kit whose `runtime.listen` is `listen`.
    fn write_kit_listening_on(dir: &Path, listen: &str) -> std::path::PathBuf {
        let yaml = format!(
            r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
runtime:
  listen: "{listen}"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {{}}
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
        );
        let path = dir.join("agent.swarmkit.yaml");
        std::fs::write(&path, yaml).unwrap();
        path
    }

    /// Resolve a start of `arkavo agent <values>` in `cwd`, with the kit at
    /// `kit` when one is named.
    fn resolve_start(
        values: &[&str],
        kit: Option<&Path>,
        cwd: &Path,
    ) -> Result<StartupConfig, Box<dyn std::error::Error>> {
        let options = run_options(values);
        resolve_startup_config(
            kit,
            options.name.as_deref(),
            options.port,
            options.bind,
            cwd,
        )
    }

    /// Regression: `[::1]:8080` was split on `:` into more than two parts,
    /// reported as invalid, and the agent was then restarted on all
    /// interfaces under a generic persona.
    #[test]
    fn a_bracketed_ipv6_listen_address_is_bound_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let kit = write_kit_listening_on(dir.path(), "[::1]:8080");

        for values in [&[][..], &["--trust"][..]] {
            let (config, listen_addr, bind_notice) =
                resolve_start(values, Some(&kit), dir.path()).unwrap();

            assert_eq!(config.name, "agent");
            assert_eq!(listen_addr, "[::1]:8080".parse().unwrap());
            assert_eq!(bind_notice, None);
        }
    }

    #[test]
    fn an_unparseable_listen_address_stops_startup() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["localhost:8080", "127.0.0.1", "8080", "not an address"] {
            let kit = write_kit_listening_on(dir.path(), bad);
            for values in [&[][..], &["--trust"][..], &["--bind", "127.0.0.1"][..]] {
                let err = resolve_start(values, Some(&kit), dir.path())
                    .expect_err("an unparseable listen address must not resolve")
                    .to_string();
                assert!(
                    err.contains("Invalid listen address"),
                    "{bad:?} gave an unexpected error: {err}"
                );
            }
        }
    }

    /// Regression: `-p` gave an address with no host the default's host, so
    /// a `runtime.listen` that stops startup was bound once a port was named.
    #[test]
    fn a_port_override_does_not_supply_a_missing_host() {
        let dir = tempfile::tempdir().unwrap();
        let kit = write_kit_listening_on(dir.path(), ":3000");
        for values in [&["-p", "8080"][..], &["-p", "8080", "--trust"][..]] {
            let err = resolve_start(values, Some(&kit), dir.path())
                .expect_err("a listen address with no host must not resolve")
                .to_string();
            assert!(err.contains("Invalid listen address"), "{err}");
        }
    }

    #[test]
    fn a_start_with_no_kit_listens_on_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let (config, listen_addr, bind_notice) = resolve_start(&[], None, dir.path()).unwrap();

        assert_eq!(listen_addr, "127.0.0.1:0".parse().unwrap());
        assert_eq!(config.listen, "127.0.0.1:0");
        assert!(config.mdns_enabled);
        assert_eq!(bind_notice, None);
    }

    /// `--trust` shows the QR code and changes nothing about where the
    /// agent listens; reaching it from another device requires an explicit bind.
    #[test]
    fn a_trust_start_listens_where_a_default_start_does() {
        let dir = tempfile::tempdir().unwrap();
        let (config, listen_addr, bind_notice) =
            resolve_start(&["--trust"], None, dir.path()).unwrap();

        assert_eq!(listen_addr, "127.0.0.1:0".parse().unwrap());
        assert_eq!(config.listen, "127.0.0.1:0");
        assert!(config.mdns_enabled);
        assert_eq!(bind_notice, None);
        assert!(listen::exposure_warning(listen_addr).is_none());

        let (_, with_port, _) =
            resolve_start(&["--trust", "-p", "8343"], None, dir.path()).unwrap();
        assert_eq!(with_port, "127.0.0.1:8343".parse().unwrap());
    }

    #[test]
    fn a_trust_start_keeps_the_address_in_the_kit() {
        let dir = tempfile::tempdir().unwrap();
        for kit_listen in ["10.0.0.140:8342", "0.0.0.0:8342", "127.0.0.1:8342"] {
            let kit = write_kit_listening_on(dir.path(), kit_listen);
            let (config, listen_addr, bind_notice) =
                resolve_start(&["--trust"], Some(&kit), dir.path()).unwrap();

            assert_eq!(listen_addr, kit_listen.parse().unwrap());
            assert_eq!(config.listen, kit_listen);
            assert_eq!(bind_notice, None, "{kit_listen}");
        }
    }

    #[test]
    fn a_port_alone_never_changes_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let (_, no_kit, _) = resolve_start(&["-p", "8343"], None, dir.path()).unwrap();
        assert_eq!(no_kit, "127.0.0.1:8343".parse().unwrap());

        let kit = write_kit_listening_on(dir.path(), "127.0.0.1:8342");
        let (_, with_kit, bind_notice) =
            resolve_start(&["-p", "8343"], Some(&kit), dir.path()).unwrap();
        assert_eq!(with_kit, "127.0.0.1:8343".parse().unwrap());
        assert_eq!(bind_notice, None);
    }

    #[test]
    fn a_bind_start_listens_on_the_named_host() {
        let dir = tempfile::tempdir().unwrap();
        for (values, listens_on) in [
            (&["--bind", "127.0.0.1"][..], "127.0.0.1:0"),
            (&["--bind", "127.0.0.1", "-p", "8343"][..], "127.0.0.1:8343"),
            (&["--bind", "127.0.0.1:8342"][..], "127.0.0.1:8342"),
            (&["--bind", "[::1]:8342"][..], "[::1]:8342"),
            (&["--bind", "127.0.0.1", "--trust"][..], "127.0.0.1:0"),
            (&["--bind", "0.0.0.0"][..], "0.0.0.0:0"),
        ] {
            let (config, listen_addr, bind_notice) =
                resolve_start(values, None, dir.path()).unwrap();

            assert_eq!(listen_addr, listens_on.parse().unwrap(), "{values:?}");
            assert_eq!(config.listen, listens_on, "{values:?}");
            assert!(config.mdns_enabled, "{values:?}");
            assert_eq!(bind_notice, None, "{values:?}");
        }
    }

    #[test]
    fn a_bind_start_sets_aside_the_address_in_the_kit_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        for (kit_listen, values, listens_on) in [
            (
                "10.0.0.140:8342",
                &["--bind", "127.0.0.1"][..],
                "127.0.0.1:8342",
            ),
            ("127.0.0.1:8342", &["--bind", "0.0.0.0"][..], "0.0.0.0:8342"),
            (
                "10.0.0.140:8342",
                &["--bind", "127.0.0.1", "-p", "9000"][..],
                "127.0.0.1:9000",
            ),
        ] {
            let kit = write_kit_listening_on(dir.path(), kit_listen);
            let (_, listen_addr, bind_notice) =
                resolve_start(values, Some(&kit), dir.path()).unwrap();

            assert_eq!(listen_addr, listens_on.parse().unwrap(), "{values:?}");
            let notice = bind_notice.expect("the kit's address was set aside");
            assert_eq!(notice.lines().count(), 1, "{notice}");
            assert!(notice.contains("--bind"), "{notice}");
            assert!(notice.contains(kit_listen), "{notice}");
            assert!(notice.contains(listens_on), "{notice}");
        }
    }

    /// The server entry point refuses the address itself, as its first step,
    /// so no caller can reach a bind with an address that did not parse.
    #[tokio::test]
    #[allow(clippy::disallowed_methods)]
    async fn start_agent_server_refuses_an_unparseable_listen_address() {
        let config = AgentConfig {
            name: "agent".to_string(),
            listen: "[::1".to_string(),
            ..AgentConfig::default()
        };

        let err = start_agent_server(&config, false)
            .await
            .expect_err("an unparseable listen address must not start a server")
            .to_string();

        assert!(err.contains("Invalid listen address"), "{err}");
    }
}
