use arkavo_protocol::{McpConnectionTrait, get_service_ip};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        print_usage();
        return Ok(());
    }

    match args[0].as_str() {
        "init" => {
            if args.len() < 2 {
                eprintln!("Error: Agent name required");
                eprintln!("Usage: arkavo agent init <agent-name>");
                return Err("Missing agent name".into());
            }
            init_agent(&args[1])
        }
        "run" => run_agent(args.get(1).map(|s| s.as_str())),
        "help" | "-h" | "--help" => {
            print_usage();
            Ok(())
        }
        _ => {
            eprintln!("Error: Unknown agent subcommand '{}'", args[0]);
            print_usage();
            Err(format!("Unknown subcommand: {}", args[0]).into())
        }
    }
}

fn print_usage() {
    println!("Arkavo Agent - Configure and run AI agents");
    println!();
    println!("USAGE:");
    println!("    arkavo agent <SUBCOMMAND> [OPTIONS]");
    println!();
    println!("SUBCOMMANDS:");
    println!("    init <name>    Create a new AGENTS.md configuration file");
    println!("    run [config]   Run an agent using AGENTS.md (or specified config)");
    println!("    help           Print this help message");
}

// Extract agent role/purpose from AGENTS.md for use in chat mode
pub fn extract_agent_role() -> Option<String> {
    if let Ok(content) = fs::read_to_string("AGENTS.md")
        && let Ok(agents) = parse_agents_config(&content)
        && let Some(first_agent) = agents.first()
    {
        return Some(first_agent.purpose.clone());
    }
    None
}

fn init_agent(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let agents_path = Path::new("AGENTS.md");

    if agents_path.exists() {
        return Err("AGENTS.md already exists. Please rename or remove it first.".into());
    }

    // Create a basic AGENTS.md template
    let template = format!(
        "# AGENTS.md\n\n## {name}\n\npurpose: \"Agent purpose\"\nmodel: \"ollama://127.0.0.1:11434/qwen3:0.6b\"\nlisten: \"0.0.0.0:8342\""
    );

    fs::write(agents_path, template)?;
    println!("Created AGENTS.md with agent configuration for '{name}'");
    println!("Edit AGENTS.md to customize your agent, then run:");
    println!("  arkavo agent run");

    Ok(())
}

#[allow(clippy::disallowed_methods)]
fn run_agent(config_path: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    use crate::commands::agent;
    use std::env;

    let config_file = config_path.unwrap_or("AGENTS.md");
    let config_path = Path::new(config_file);

    if !config_path.exists() {
        // Auto-generate AGENTS.md with directory-based naming

        // Get current directory name
        let current_dir = env::current_dir()?;
        let dir_name = current_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        // Generate a short random ID from UUID (first 7 chars)
        use uuid::Uuid;
        let random_id = &Uuid::new_v4().to_string()[..7];

        // Create agent name: directory-randomid
        let agent_name = format!("{dir_name}-{random_id}");

        // Generate AGENTS.md with defaults using embedded template
        let template_content = crate::prompt_loader::load_prompt(
            "agents_md",
            "# AGENTS.md\n\n## {name}\n\npurpose: \"Agent purpose\"\nmodel: \"ollama://127.0.0.1:11434/qwen3:0.6b\"\nlisten: \"0.0.0.0:8342\"",
        );
        let template = template_content.replace("{name}", &agent_name);

        fs::write(config_path, template)?;
        println!("Auto-generated AGENTS.md with agent '{agent_name}'");
    }

    let config_content = fs::read_to_string(config_path)?;

    // Parse the AGENTS.md file
    let agents = agent::parse_agents_config(&config_content)?;

    if agents.is_empty() {
        return Err("No agent configurations found in file".into());
    }

    // If multiple agents are defined, let user select which one to run
    let agent_config = if agents.len() == 1 {
        &agents[0]
    } else {
        println!("Multiple agents found in configuration:");
        for (i, agent) in agents.iter().enumerate() {
            println!("  {}: {} - {}", i + 1, agent.name, agent.purpose);
        }

        print!("Select agent to run (1-{}): ", agents.len());

        // Read user input
        use std::io::{self, Write};
        print!("Enter selection: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let selection: usize = input
            .trim()
            .parse()
            .map_err(|_| "Invalid selection. Please enter a number.")?;

        if selection == 0 || selection > agents.len() {
            return Err(format!(
                "Selection {} is out of range. Please select 1-{}",
                selection,
                agents.len()
            )
            .into());
        }

        &agents[selection - 1]
    };

    println!("Starting agent: {}", agent_config.name);
    println!("Purpose: {}", agent_config.purpose);
    println!("Model: {}", agent_config.model);
    println!("Listen: {}", agent_config.listen);
    println!("mDNS enabled: {}", agent_config.mdns_enabled);

    if !agent_config.mcp_servers.is_empty() {
        println!("MCP servers:");
        for server in &agent_config.mcp_servers {
            println!("  - {}: ", server.name);
            if let Some(cmd) = &server.command {
                println!("    command: {} {:?}", cmd, server.args);
            } else if let Some(url) = &server.url {
                println!("    url: {url}");
            }
        }
    }

    // Start the A2A server with the agent configuration
    // Check if we're already in a runtime context
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // Already in a runtime, use the existing handle
            handle.block_on(async { agent::start_agent_server(agent_config).await })
        }
        Err(_) => {
            // Not in a runtime, create one
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async { agent::start_agent_server(agent_config).await })
        }
    }
}

// Agent configuration parsing
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub purpose: String,
    pub model: String,
    pub listen: String,
    pub mdns_enabled: bool,
    pub mcp_servers: Vec<McpServerConfig>,
    pub api_keys: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}

pub fn parse_agents_config(content: &str) -> Result<Vec<AgentConfig>, Box<dyn std::error::Error>> {
    let mut agents = Vec::new();
    let mut current_agent: Option<AgentConfig> = None;
    let mut in_agent_section = false;
    let mut in_mcp_section = false;
    let mut current_mcp_server: Option<McpServerConfig> = None;
    let mut current_section: Option<String> = None;

    // Check if this is the new markdown format by looking for specific patterns
    let is_new_format = content.contains("## Agent Identity")
        || content.contains("**Mission:**")
        || content.contains("**Name:**")
        || content.contains("# AGENTS.md");

    for line in content.lines() {
        let trimmed = line.trim();

        // Handle new markdown format
        if is_new_format {
            // Extract agent name from H1 title (e.g., "# AGENTS.md — alert-manager")
            if trimmed.starts_with("# AGENTS.md") && trimmed.contains("—") {
                // Save previous agent if exists
                if let Some(agent) = current_agent.take() {
                    agents.push(agent);
                }

                let name = if let Some(em_dash_pos) = trimmed.find("—") {
                    trimmed[em_dash_pos + "—".len()..].trim().to_string()
                } else if let Some(dash_pos) = trimmed.find(" - ") {
                    trimmed[dash_pos + 3..].trim().to_string()
                } else {
                    "unnamed-agent".to_string()
                };

                current_agent = Some(AgentConfig {
                    name,
                    purpose: String::new(),
                    model: "ollama://127.0.0.1:11434/qwen3:0.6b".to_string(), // Default model
                    listen: "0.0.0.0:8342".to_string(), // Default listen address
                    mdns_enabled: true,
                    mcp_servers: Vec::new(),
                    api_keys: std::collections::HashMap::new(),
                });
                in_agent_section = true;
                continue;
            }

            // Track current markdown section
            if trimmed.starts_with("## ") {
                let section_name = trimmed.strip_prefix("## ").unwrap_or("").to_string();

                // Check if this is an agent definition (not a standard section like "Agent Identity")
                if !matches!(
                    section_name.as_str(),
                    "Agent Identity"
                        | "Runtime Configuration"
                        | "Runtime Configuration (example)"
                        | "Capabilities"
                        | "Tool Requirements"
                        | "MCP Servers"
                ) {
                    // Save any pending MCP server before switching agents
                    if let (Some(server), Some(agent)) =
                        (current_mcp_server.take(), current_agent.as_mut())
                    {
                        agent.mcp_servers.push(server);
                    }

                    // Save previous agent if exists
                    if let Some(agent) = current_agent.take() {
                        agents.push(agent);
                    }

                    // Reset MCP section flag
                    in_mcp_section = false;

                    // Create new agent from ## agent-name header
                    current_agent = Some(AgentConfig {
                        name: section_name.clone(),
                        purpose: String::new(),
                        model: "ollama://127.0.0.1:11434/qwen3:0.6b".to_string(), // Default model
                        listen: "0.0.0.0:8342".to_string(), // Default listen address
                        mdns_enabled: true,
                        mcp_servers: Vec::new(),
                        api_keys: std::collections::HashMap::new(),
                    });
                    in_agent_section = true;
                }

                current_section = Some(section_name);
                continue;
            }

            // Parse agent information from markdown sections
            if let Some(agent) = current_agent.as_mut() {
                if let Some(section) = &current_section {
                    match section.as_str() {
                        "Agent Identity" => {
                            // Extract name
                            if trimmed.starts_with("- **Name:**") {
                                if let Some(name) =
                                    extract_markdown_field_value(trimmed, "**Name:**")
                                {
                                    agent.name = name;
                                }
                            }
                            // Extract mission/purpose
                            else if trimmed.starts_with("- **Mission:**") {
                                if let Some(mission) =
                                    extract_markdown_field_value(trimmed, "**Mission:**")
                                {
                                    agent.purpose = mission;
                                }
                            }
                        }
                        "Runtime Configuration (example)" => {
                            // Try to extract listen address from YAML block
                            if trimmed.starts_with("listen:") {
                                if let Some(listen) = extract_yaml_value(trimmed, "listen:") {
                                    agent.listen = listen;
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Also handle direct YAML-style properties in markdown (for flexibility)
                parse_yaml_properties(trimmed, agent, &mut in_mcp_section, &mut current_mcp_server);
            }
        } else {
            // Handle old YAML-style format
            // Check for agent section header
            if trimmed.starts_with("## ") {
                // Save any pending MCP server before switching agents
                if let (Some(server), Some(agent)) =
                    (current_mcp_server.take(), current_agent.as_mut())
                {
                    agent.mcp_servers.push(server);
                }

                // Save previous agent if exists
                if let Some(agent) = current_agent.take() {
                    agents.push(agent);
                }

                // Reset MCP section flag
                in_mcp_section = false;

                let name = trimmed.strip_prefix("## ").unwrap_or("").trim().to_string();
                current_agent = Some(AgentConfig {
                    name,
                    purpose: String::new(),
                    model: String::new(),
                    listen: String::new(),
                    mdns_enabled: true, // Default to true for zero-config
                    mcp_servers: Vec::new(),
                    api_keys: std::collections::HashMap::new(),
                });
                in_agent_section = true;
                continue;
            }

            // Skip if not in agent section
            if !in_agent_section || current_agent.is_none() {
                continue;
            }

            if let Some(agent) = current_agent.as_mut() {
                parse_yaml_properties(trimmed, agent, &mut in_mcp_section, &mut current_mcp_server);
            }
        }
    }

    // Save any pending MCP server
    if let (Some(server), Some(agent)) = (current_mcp_server.take(), current_agent.as_mut()) {
        agent.mcp_servers.push(server);
    }

    // Save last agent
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}

// Helper function to extract value from markdown field like "- **Name:** value"
fn extract_markdown_field_value(line: &str, field_prefix: &str) -> Option<String> {
    if let Some(start_pos) = line.find(field_prefix) {
        let after_prefix = &line[start_pos + field_prefix.len()..];
        let value = after_prefix.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

// Helper function to extract YAML-style values like "listen: 0.0.0.0:8342"
fn extract_yaml_value(line: &str, key: &str) -> Option<String> {
    if let Some(colon_pos) = line.find(':') {
        let key_part = line[..colon_pos].trim();
        if key_part == key.trim_end_matches(':') {
            let value = line[colon_pos + 1..].trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

// Helper function to parse YAML-style properties (used by both old and new format parsers)
fn parse_yaml_properties(
    trimmed: &str,
    agent: &mut AgentConfig,
    in_mcp_section: &mut bool,
    current_mcp_server: &mut Option<McpServerConfig>,
) {
    // Check for mcp_servers section
    if trimmed == "mcp_servers:" {
        *in_mcp_section = true;
        return;
    }

    // Handle MCP server entries
    if *in_mcp_section && trimmed.starts_with("- name:") {
        // Save previous MCP server if exists
        if let Some(server) = current_mcp_server.take() {
            agent.mcp_servers.push(server);
        }

        // Start new MCP server
        *current_mcp_server = Some(McpServerConfig {
            name: trimmed
                .strip_prefix("- name:")
                .unwrap_or("")
                .trim()
                .to_string(),
            command: None,
            args: Vec::new(),
            url: None,
        });
        return;
    }

    // Parse MCP server properties
    if *in_mcp_section && let Some(server) = current_mcp_server.as_mut() {
        if trimmed.starts_with("command:") {
            server.command = Some(
                trimmed
                    .strip_prefix("command:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
        } else if trimmed.starts_with("args:") {
            // Parse array format: ["arg1", "arg2"]
            let args_str = trimmed.strip_prefix("args:").unwrap_or("").trim();
            if args_str.starts_with('[') && args_str.ends_with(']') {
                let args_content = &args_str[1..args_str.len() - 1];
                server.args = args_content
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        } else if trimmed.starts_with("url:") {
            server.url = Some(
                trimmed
                    .strip_prefix("url:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
        } else if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('-') {
            // End of MCP section
            *in_mcp_section = false;
            if let Some(server) = current_mcp_server.take() {
                agent.mcp_servers.push(server);
            }
        }
    }

    // Parse agent properties (when not in MCP section)
    if !*in_mcp_section {
        if trimmed.starts_with("purpose:") {
            agent.purpose = trimmed
                .strip_prefix("purpose:")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if trimmed.starts_with("model:") {
            agent.model = trimmed
                .strip_prefix("model:")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if trimmed.starts_with("listen:") {
            agent.listen = trimmed
                .strip_prefix("listen:")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if trimmed.starts_with("mdns:") {
            // Only disable if explicitly set to false
            agent.mdns_enabled = !trimmed.contains("false");
        } else if trimmed.contains("_API_KEY:") || trimmed.contains("_api_key:") {
            // Parse API key entries (e.g., MOONSHOT_API_KEY: sk-xxx)
            if let Some(colon_pos) = trimmed.find(':') {
                let key_name = trimmed[..colon_pos].trim().to_string();
                let key_value = trimmed[colon_pos + 1..]
                    .trim()
                    .trim_matches('"')
                    .to_string();
                agent.api_keys.insert(key_name, key_value);
            }
        }
    }
}

#[allow(clippy::future_not_send)]
pub async fn start_agent_server(config: &AgentConfig) -> Result<(), Box<dyn std::error::Error>> {
    use crate::mcp_spawner::McpProcessManager;
    use arkavo_protocol::{config::ServerConfig, rate_limit::RateLimitConfig, server::A2aServer};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Create process manager for MCP servers
    let process_manager = McpProcessManager::new();

    // Create shutdown flag for mDNS thread
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    // Parse listen address
    let parts: Vec<&str> = config.listen.split(':').collect();
    if parts.len() != 2 {
        return Err("Invalid listen address format. Expected: host:port".into());
    }

    // Use absolute path for task store to avoid issues with directory changes
    let task_store_path = std::env::current_dir()?
        .join(".arkavo")
        .join("arkavo_tasks.db");

    let server_config = ServerConfig {
        enabled: true,
        bind_address: parts[0].to_string(),
        port: parts[1].parse()?,
        max_connections: 100,
        idle_timeout_seconds: 300,
        rate_limit: RateLimitConfig::default(),
        task_store_path: Some(task_store_path.to_string_lossy().to_string()),
        metrics_enabled: true,
    };

    let server = A2aServer::new(server_config);

    // Set agent metadata
    server
        .set_agent_metadata(
            config.name.clone(),
            config.purpose.clone(),
            config.model.clone(),
        )
        .await;

    // Set API keys in the server
    server.set_api_keys(config.api_keys.clone()).await;

    // Initialize MCP connections
    let mcp_registry = server.mcp_registry();

    // Register built-in MCP tools first
    {
        use crate::builtin_mcp::BuiltinMcpConnection;
        #[cfg(all(target_os = "macos", feature = "test-harness"))]
        let builtin_connection = BuiltinMcpConnection::new_with_test_tools().await;
        #[cfg(not(all(target_os = "macos", feature = "test-harness")))]
        let builtin_connection = BuiltinMcpConnection::new_with_test_tools();
        mcp_registry
            .register("arkavo".to_string(), Box::new(builtin_connection))
            .await;
        println!("Registered Arkavo MCP tools (runtime and test tools)");
    }

    for mcp_config in &config.mcp_servers {
        println!("Initializing MCP server: {}", mcp_config.name);

        // Create appropriate MCP connection based on config
        if let Some(command) = &mcp_config.command {
            // Create MCP client using the existing sync approach
            use crate::mcp_client::McpClient;
            use crate::mcp_integration::McpConnection;

            match McpClient::new_with_command(command, &mcp_config.args) {
                Ok(client) => {
                    // Register the spawned process with the process manager for cleanup
                    // Note: McpClient handles its own process lifecycle, but we track it for coordinated shutdown
                    let pid = if let Ok(process) = client.process.lock() {
                        let pid = process.child.id();
                        process_manager.register_process(mcp_config.name.clone(), pid);
                        pid
                    } else {
                        0
                    };

                    // Telemetry: MCP server started
                    println!(
                        "[INFO] mcp.server.started name={} command={} pid={} args={:?}",
                        mcp_config.name, command, pid, mcp_config.args
                    );

                    let connection = McpConnection::External(client);
                    let wrapped = McpConnectionWrapper::new(connection);
                    mcp_registry
                        .register(mcp_config.name.clone(), Box::new(wrapped))
                        .await;
                    println!(
                        "Started command-based MCP server: {} ({})",
                        mcp_config.name, command
                    );
                }
                Err(e) => {
                    // Telemetry: MCP server failed to start (non-fatal)
                    println!(
                        "[WARN] mcp.server.start_failed name={} command={} error=\"{}\"",
                        mcp_config.name, command, e
                    );
                    eprintln!(
                        "Warning: MCP server '{}' not available ({})",
                        mcp_config.name, command
                    );
                    eprintln!("  Agent will continue with reduced capabilities.");
                    if command == "mcp-filesystem" || command == "mcp-git" {
                        eprintln!(
                            "  To install: npm install -g @modelcontextprotocol/server-{}",
                            if command == "mcp-filesystem" {
                                "filesystem"
                            } else {
                                "git"
                            }
                        );
                    }
                }
            }
        } else if let Some(url) = &mcp_config.url {
            // Create external MCP connection
            use crate::mcp_integration::McpConnection;
            match McpConnection::new_external(Some(url.clone())) {
                Ok(connection) => {
                    // We need to wrap the connection to implement the trait
                    let wrapped = McpConnectionWrapper::new(connection);
                    mcp_registry
                        .register(mcp_config.name.clone(), Box::new(wrapped))
                        .await;
                    println!(
                        "Connected to external MCP server: {} at {}",
                        mcp_config.name, url
                    );
                }
                Err(e) => {
                    eprintln!("Failed to connect to MCP server {}: {}", mcp_config.name, e);
                }
            }
        }
    }

    let handle = server.start().await?;

    // Give the server a moment to fully initialize
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Start mDNS broadcasting if enabled
    let mdns_thread_handle = if config.mdns_enabled {
        println!("Starting mDNS broadcasting...");
        let config_clone = config.clone();
        let shutdown_flag_clone = shutdown_flag.clone();
        // Use std::thread since zeroconf is not Send
        let handle = std::thread::spawn(move || {
            println!("mDNS thread spawned");
            if let Err(e) = broadcast_agent_mdns_sync(&config_clone, shutdown_flag_clone) {
                eprintln!("mDNS broadcast error: {e}");
            }
        });
        // Give the mDNS thread a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        Some(handle)
    } else {
        println!("mDNS broadcasting disabled");
        None
    };

    println!("Agent server started on {}", config.listen);
    println!("Press Ctrl+C to stop");

    // Keep the server running
    tokio::signal::ctrl_c().await?;

    println!("Shutting down agent server...");

    // Signal the mDNS thread to stop
    shutdown_flag.store(true, Ordering::Relaxed);

    // Stop the A2A server
    handle.stop()?;

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

    // Explicitly exit to ensure all threads are terminated
    std::process::exit(0);
}

fn broadcast_agent_mdns_sync(
    #[allow(unused_variables)] config: &AgentConfig,
    #[allow(unused_variables)] shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mdns")]
    {
        use mdns_sd::{ServiceDaemon, ServiceInfo};
        use std::collections::HashMap;
        use std::thread;
        use std::time::Duration;

        println!("broadcast_agent_mdns: Starting for agent '{}'", config.name);

        let port: u16 = config.listen.split(':').nth(1).unwrap().parse()?;
        println!("broadcast_agent_mdns: Parsed port: {port}");

        // Get the actual network IP for advertising instead of 0.0.0.0
        let service_ip = get_service_ip();
        println!("broadcast_agent_mdns: Using IP address: {service_ip}");

        // Create mDNS daemon
        let mdns = ServiceDaemon::new()?;

        // Start browsing for other agents
        let receiver = mdns.browse("_a2a._tcp.local.")?;

        // Clone shutdown flag for discovery thread
        let shutdown_flag_discovery = shutdown_flag.clone();

        // Spawn a thread to handle discovered services
        let discovery_thread = thread::spawn(move || {
            use mdns_sd::ServiceEvent;
            println!("Starting discovery of other agents...");

            loop {
                match receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(event) => match event {
                        ServiceEvent::ServiceResolved(info) => {
                            println!("Agent discovered: {}", info.get_fullname());
                            if let Some(agent_id) = info.get_property_val_str("agent_id") {
                                println!("  - Agent ID: {agent_id}");
                            }
                            if let Some(purpose) = info.get_property_val_str("purpose") {
                                println!("  - Purpose: {purpose}");
                            }
                            if let Some(addr) = info.get_addresses().iter().next() {
                                println!("  - Address: {}:{}", addr, info.get_port());
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            println!("Agent disconnected: {fullname}");
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

        // Prepare properties
        let mut properties = HashMap::new();
        properties.insert("agent_id".to_string(), config.name.clone());
        properties.insert("purpose".to_string(), config.purpose.clone());
        properties.insert("model".to_string(), config.model.clone());
        properties.insert("ip".to_string(), service_ip.to_string());

        // Add capabilities based on agent name/purpose
        let capabilities = get_agent_capabilities(&config.name, &config.purpose);
        if !capabilities.is_empty() {
            properties.insert("capabilities".to_string(), capabilities.join(","));
        }

        // Add MCP servers as capabilities
        if !config.mcp_servers.is_empty() {
            let mcp_tools: Vec<String> =
                config.mcp_servers.iter().map(|s| s.name.clone()).collect();
            properties.insert("mcp_tools".to_string(), mcp_tools.join(","));
        }

        // Create service info
        let service_type = "_a2a._tcp.local.";
        let instance_name = format!("arkavo-agent-{}", config.name);
        let host_name = format!("{}.local.", config.name);

        let service_info = ServiceInfo::new(
            service_type,
            &instance_name,
            &host_name,
            service_ip.to_string(),
            port,
            properties,
        )?;

        // Register the service
        mdns.register(service_info)?;

        println!("mDNS service registered successfully!");
        println!("Service name: arkavo-agent-{}", config.name);
        println!("Service type: _a2a._tcp");
        println!("Host: {service_ip}");
        println!("Port: {port}");
        println!("WebSocket endpoint: ws://{service_ip}:{port}/ws");

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
        println!("mDNS support not compiled in");
        println!("Agent will run without mDNS discovery");
        // Keep the thread alive until shutdown is signaled
        use std::sync::atomic::Ordering;
        while !shutdown_flag.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    Ok(())
}

// Wrapper to implement McpConnectionTrait for arkavo-cli's McpConnection
struct McpConnectionWrapper {
    inner: crate::mcp_integration::McpConnection,
}

impl McpConnectionWrapper {
    fn new(connection: crate::mcp_integration::McpConnection) -> Self {
        Self { inner: connection }
    }
}

impl McpConnectionTrait for McpConnectionWrapper {
    fn list_tools(
        &self,
    ) -> Result<Vec<arkavo_protocol::mcp_registry::Tool>, Box<dyn std::error::Error>> {
        // Convert from cli Tool to protocol Tool
        let cli_tools = self.inner.list_tools()?;
        let protocol_tools = cli_tools
            .into_iter()
            .map(|t| arkavo_protocol::mcp_registry::Tool {
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
    ) -> Result<Value, Box<dyn std::error::Error>> {
        self.inner.call_tool(tool_name, arguments, llm_provider)
    }
}

/// Determine agent capabilities based on name and purpose
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
