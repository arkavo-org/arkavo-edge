use log::info;
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
discovery:
  mdns: true

# Additional agent configurations can be added below
# Each agent starts with ## agent-name

# Example configurations:
#
# ## code-reviewer
# purpose: Review code for quality and suggest improvements
# model:   openai://gpt-4
# listen:  0.0.0.0:8343
# discovery:
#   mdns: true
#
# ## test-runner
# purpose: Run tests and report results
# model:   anthropic://claude-3-opus
# listen:  0.0.0.0:8344
# discovery:
#   mdns: false
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
                mdns_enabled: false,
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
            } else if trimmed.starts_with("mdns:") && trimmed.contains("true") {
                agent.mdns_enabled = true;
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

    // Start mDNS broadcasting if enabled
    if config.mdns_enabled {
        let config_clone = config.clone();
        tokio::spawn(async move {
            if let Err(e) = broadcast_agent_mdns(&config_clone).await {
                eprintln!("mDNS broadcast error: {}", e);
            }
        });
    }

    println!("Agent server started on {}", config.listen);
    println!("Press Ctrl+C to stop");

    // Keep the server running
    tokio::signal::ctrl_c().await?;
    handle.stop()?;

    Ok(())
}

async fn broadcast_agent_mdns(config: &AgentConfig) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::mdns::{MdnsManager, MdnsServiceInfo};

    let port: u16 = config.listen.split(':').nth(1).unwrap().parse()?;

    let service_info = MdnsServiceInfo {
        agent_id: config.name.clone(),
        http_port: port,
        ws_port: None,
        version: "0.1.0".to_string(),
        capabilities: vec!["promise_request".to_string(), "rpc.discover".to_string()],
    };

    let mdns_manager = MdnsManager::new();
    mdns_manager.register_service(service_info).await?;

    info!("mDNS service registered for agent: {}", config.name);

    // Keep the mDNS service running
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}
