use std::fs;
use std::path::Path;

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

fn init_agent(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let agents_path = Path::new("AGENTS.md");

    if agents_path.exists() {
        return Err("AGENTS.md already exists. Please rename or remove it first.".into());
    }

    let template = format!(
        r#"# AGENTS.md

## {}
purpose: Describe what this agent does
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8342

# mDNS discovery is enabled by default for zero-config networking
# To disable mDNS, uncomment the following:
# discovery:
#   mdns: false

# Additional agent configurations can be added below
# Each agent starts with ## agent-name

# Example configurations:
#
# ## code-reviewer
# purpose: Review code for quality and suggest improvements
# model:   openai://gpt-4
# listen:  0.0.0.0:8343
#
# ## test-runner
# purpose: Run tests and report results
# model:   anthropic://claude-3-opus
# listen:  0.0.0.0:8344
# discovery:
#   mdns: false  # Explicitly disable mDNS for this agent
"#,
        name
    );

    fs::write(agents_path, template)?;
    println!("Created AGENTS.md with agent configuration for '{}'", name);
    println!("Edit AGENTS.md to customize your agent, then run:");
    println!("  arkavo agent run");

    Ok(())
}

fn run_agent(config_path: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    use crate::commands::agent;

    let config_file = config_path.unwrap_or("AGENTS.md");
    let config_path = Path::new(config_file);

    if !config_path.exists() {
        eprintln!("Error: Configuration file '{}' not found", config_file);
        eprintln!("Run 'arkavo agent init <name>' to create one");
        return Err(format!("Config file not found: {}", config_file).into());
    }

    let config_content = fs::read_to_string(config_path)?;

    // Parse the AGENTS.md file
    let agents = agent::parse_agents_config(&config_content)?;

    if agents.is_empty() {
        return Err("No agent configurations found in file".into());
    }

    // For now, run the first agent found
    // TODO: Add agent selection if multiple agents are defined
    let agent_config = &agents[0];

    println!("Starting agent: {}", agent_config.name);
    println!("Purpose: {}", agent_config.purpose);
    println!("Model: {}", agent_config.model);
    println!("Listen: {}", agent_config.listen);
    println!("mDNS enabled: {}", agent_config.mdns_enabled);

    // Start the A2A server with the agent configuration
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async { agent::start_agent_server(agent_config).await })
}

// Agent configuration parsing
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub purpose: String,
    pub model: String,
    pub listen: String,
    pub mdns_enabled: bool,
}

pub fn parse_agents_config(content: &str) -> Result<Vec<AgentConfig>, Box<dyn std::error::Error>> {
    let mut agents = Vec::new();
    let mut current_agent: Option<AgentConfig> = None;
    let mut in_agent_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for agent section header
        if trimmed.starts_with("## ") {
            // Save previous agent if exists
            if let Some(agent) = current_agent.take() {
                agents.push(agent);
            }

            let name = trimmed[3..].trim().to_string();
            current_agent = Some(AgentConfig {
                name,
                purpose: String::new(),
                model: String::new(),
                listen: String::new(),
                mdns_enabled: true, // Default to true for zero-config
            });
            in_agent_section = true;
            continue;
        }

        // Skip if not in agent section
        if !in_agent_section || current_agent.is_none() {
            continue;
        }

        // Parse agent properties
        if let Some(agent) = current_agent.as_mut() {
            if trimmed.starts_with("purpose:") {
                agent.purpose = trimmed[8..].trim().to_string();
            } else if trimmed.starts_with("model:") {
                agent.model = trimmed[6..].trim().to_string();
            } else if trimmed.starts_with("listen:") {
                agent.listen = trimmed[7..].trim().to_string();
            } else if trimmed.starts_with("mdns:") {
                // Only disable if explicitly set to false
                agent.mdns_enabled = !trimmed.contains("false");
            }
        }
    }

    // Save last agent
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}

pub async fn start_agent_server(config: &AgentConfig) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::{config::ServerConfig, rate_limit::RateLimitConfig, server::A2aServer};

    // Parse listen address
    let parts: Vec<&str> = config.listen.split(':').collect();
    if parts.len() != 2 {
        return Err("Invalid listen address format. Expected: host:port".into());
    }

    let server_config = ServerConfig {
        enabled: true,
        bind_address: parts[0].to_string(),
        port: parts[1].parse()?,
        max_connections: 100,
        idle_timeout_seconds: 300,
        rate_limit: RateLimitConfig::default(),
    };

    let server = A2aServer::new(server_config);
    let handle = server.start().await?;

    // Give the server a moment to fully initialize
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Start mDNS broadcasting if enabled
    if config.mdns_enabled {
        println!("Starting mDNS broadcasting...");
        let config_clone = config.clone();
        // Use std::thread since zeroconf is not Send
        std::thread::spawn(move || {
            println!("mDNS thread spawned");
            if let Err(e) = broadcast_agent_mdns_sync(&config_clone) {
                eprintln!("mDNS broadcast error: {}", e);
            }
        });
        // Give the mDNS thread a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    } else {
        println!("mDNS broadcasting disabled");
    }

    println!("Agent server started on {}", config.listen);
    println!("Press Ctrl+C to stop");

    // Keep the server running
    tokio::signal::ctrl_c().await?;
    handle.stop()?;

    Ok(())
}

fn broadcast_agent_mdns_sync(_config: &AgentConfig) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mdns")]
    let config = _config;
    #[cfg(feature = "mdns")]
    {
        use std::collections::HashMap;
        use std::thread;
        use std::time::Duration;
        use zeroconf::{MdnsService, ServiceType, TxtRecord, prelude::*};

        println!("broadcast_agent_mdns: Starting for agent '{}'", config.name);

        let port: u16 = config.listen.split(':').nth(1).unwrap().parse()?;
        println!("broadcast_agent_mdns: Parsed port: {port}");

        let mut txt = TxtRecord::new();
        let mut properties = HashMap::new();
        properties.insert("agent_id", config.name.clone());
        properties.insert("purpose", config.purpose.clone());
        properties.insert("model", config.model.clone());

        for (key, value) in &properties {
            txt.insert(key, value)?;
        }

        let mut service = MdnsService::new(ServiceType::new("a2a", "tcp")?, port);
        service.set_name(&format!("arkavo-agent-{}", config.name));
        service.set_txt_record(txt);

        let service = service.register()?;

        println!("mDNS service registered successfully!");
        println!("Service name: arkavo-agent-{}", config.name);
        println!("Service type: _a2a._tcp");
        println!("Port: {}", port);
        println!("Check with: dns-sd -B _a2a._tcp local.");

        // The service automatically unregisters when it goes out of scope.
        // We need to keep it alive.
        loop {
            thread::sleep(Duration::from_secs(30));
            println!("mDNS service still broadcasting...");
            // Keep reference to prevent dropping
            let _ = &service;
        }
    }

    #[cfg(not(feature = "mdns"))]
    {
        println!("mDNS support not compiled in (disabled for musl builds)");
        println!("Agent will run without mDNS discovery");
        // Keep the thread alive so the agent doesn't exit
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }
}
