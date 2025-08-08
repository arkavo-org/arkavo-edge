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

fn init_agent(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let agents_path = Path::new("AGENTS.md");

    if agents_path.exists() {
        return Err("AGENTS.md already exists. Please rename or remove it first.".into());
    }

    let template = format!(
        r#"# AGENTS.md

## {name}
purpose: Describe what this agent does
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8342

# MCP servers provide additional tools and capabilities to the agent
# Uncomment and configure the following to add MCP servers:
# mcp_servers:
#   - name: filesystem
#     command: mcp-filesystem
#     args: ["--allow-write"]
#   - name: git
#     command: mcp-git
#     args: []
#   - name: external
#     url: http://localhost:8080

# mDNS discovery is enabled by default for zero-config networking
# To disable mDNS, uncomment the following:
# discovery:
#   mdns: false

# Additional agent configurations can be added below
# Each agent starts with ## agent-name

# API keys can be configured per agent (will be disseminated from UI):
# MOONSHOT_API_KEY: sk-your-api-key-here
# OPENAI_API_KEY: sk-your-openai-key
# ANTHROPIC_API_KEY: sk-your-anthropic-key

# Example configurations:
#
# ## code-reviewer
# purpose: Review code for quality and suggest improvements
# model:   openai://gpt-4
# listen:  0.0.0.0:8343
# OPENAI_API_KEY: sk-your-openai-key
# mcp_servers:
#   - name: git
#     command: mcp-git
#     args: ["--read-only"]
#
# ## test-runner
# purpose: Run tests and report results
# model:   anthropic://claude-3-opus
# listen:  0.0.0.0:8344
# ANTHROPIC_API_KEY: sk-your-anthropic-key
# discovery:
#   mdns: false  # Explicitly disable mDNS for this agent
#
# ## kimi-assistant
# purpose: AI assistant with 128k context window
# model:   kimi://moonshot-v1-128k
# listen:  0.0.0.0:8345
# MOONSHOT_API_KEY: sk-your-moonshot-key
"#
    );

    fs::write(agents_path, template)?;
    println!("Created AGENTS.md with agent configuration for '{name}'");
    println!("Edit AGENTS.md to customize your agent, then run:");
    println!("  arkavo agent run");

    Ok(())
}

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

        // Generate AGENTS.md with defaults
        let template = format!(
            r#"# AGENTS.md

## {agent_name}
purpose: AI agent for {dir_name} development
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8342
"#
        );

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

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for agent section header
        if trimmed.starts_with("## ") {
            // Save any pending MCP server before switching agents
            if let Some(server) = current_mcp_server.take()
                && let Some(agent) = current_agent.as_mut()
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

        // Check for mcp_servers section
        if trimmed == "mcp_servers:" {
            in_mcp_section = true;
            continue;
        }

        // Handle MCP server entries
        if in_mcp_section && trimmed.starts_with("- name:") {
            // Save previous MCP server if exists
            if let Some(server) = current_mcp_server.take()
                && let Some(agent) = current_agent.as_mut()
            {
                agent.mcp_servers.push(server);
            }

            // Start new MCP server
            current_mcp_server = Some(McpServerConfig {
                name: trimmed
                    .strip_prefix("- name:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                command: None,
                args: Vec::new(),
                url: None,
            });
            continue;
        }

        // Parse MCP server properties
        if in_mcp_section
            && current_mcp_server.is_some()
            && let Some(server) = current_mcp_server.as_mut()
        {
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
            } else if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('-')
            {
                // End of MCP section
                in_mcp_section = false;
                if let Some(server) = current_mcp_server.take()
                    && let Some(agent) = current_agent.as_mut()
                {
                    agent.mcp_servers.push(server);
                }
            }
        }

        // Parse agent properties (when not in MCP section)
        if !in_mcp_section && let Some(agent) = current_agent.as_mut() {
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

    // Save any pending MCP server
    if let Some(server) = current_mcp_server
        && let Some(agent) = current_agent.as_mut()
    {
        agent.mcp_servers.push(server);
    }

    // Save last agent
    if let Some(agent) = current_agent {
        agents.push(agent);
    }

    Ok(agents)
}

#[allow(clippy::future_not_send)]
pub async fn start_agent_server(config: &AgentConfig) -> Result<(), Box<dyn std::error::Error>> {
    use crate::mcp_spawner::McpProcessManager;
    use arkavo_protocol::{config::ServerConfig, rate_limit::RateLimitConfig, server::A2aServer};

    // Create process manager for MCP servers
    let process_manager = McpProcessManager::new();

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
    if config.mdns_enabled {
        println!("Starting mDNS broadcasting...");
        let config_clone = config.clone();
        // Use std::thread since zeroconf is not Send
        std::thread::spawn(move || {
            println!("mDNS thread spawned");
            if let Err(e) = broadcast_agent_mdns_sync(&config_clone) {
                eprintln!("mDNS broadcast error: {e}");
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

    println!("Shutting down agent server...");

    // Stop the A2A server
    handle.stop()?;

    // Shutdown all MCP processes
    process_manager.shutdown_all()?;

    println!("Agent server stopped.");
    Ok(())
}

fn broadcast_agent_mdns_sync(
    #[allow(unused_variables)] config: &AgentConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mdns")]
    {
        use std::collections::HashMap;
        use std::thread;
        use std::time::Duration;
        use zeroconf::{MdnsService, ServiceType, TxtRecord, prelude::*};

        println!("broadcast_agent_mdns: Starting for agent '{}'", config.name);

        let port: u16 = config.listen.split(':').nth(1).unwrap().parse()?;
        println!("broadcast_agent_mdns: Parsed port: {port}");

        // Get the actual network IP for advertising instead of 0.0.0.0
        let service_ip = get_service_ip();
        println!("broadcast_agent_mdns: Using IP address: {service_ip}");

        let mut txt = TxtRecord::new();
        let mut properties = HashMap::new();
        properties.insert("agent_id", config.name.clone());
        properties.insert("purpose", config.purpose.clone());
        properties.insert("model", config.model.clone());
        properties.insert("ip", service_ip.to_string());

        // Add capabilities based on agent name/purpose
        let capabilities = get_agent_capabilities(&config.name, &config.purpose);
        if !capabilities.is_empty() {
            properties.insert("capabilities", capabilities.join(","));
        }

        // Add MCP servers as capabilities
        if !config.mcp_servers.is_empty() {
            let mcp_tools: Vec<String> =
                config.mcp_servers.iter().map(|s| s.name.clone()).collect();
            properties.insert("mcp_tools", mcp_tools.join(","));
        }

        for (key, value) in &properties {
            txt.insert(key, value)?;
        }

        let mut service = MdnsService::new(ServiceType::new("a2a", "tcp")?, port);
        service.set_name(&format!("arkavo-agent-{}", config.name));
        service.set_txt_record(txt);
        // Note: set_host() doesn't work as expected with zeroconf library
        // The IP is provided in TXT records instead

        let service = service.register()?;

        println!("mDNS service registered successfully!");
        println!("Service name: arkavo-agent-{}", config.name);
        println!("Service type: _a2a._tcp");
        println!("Host: {service_ip}");
        println!("Port: {port}");
        println!("WebSocket endpoint: ws://{service_ip}:{port}/ws");

        // The service automatically unregisters when it goes out of scope.
        // We need to keep it alive.
        loop {
            thread::sleep(Duration::from_secs(30));
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
