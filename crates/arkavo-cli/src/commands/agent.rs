use arkavo_config_encryption::AgentIdentity;
use arkavo_protocol::get_service_ip;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Parse all arguments for flags
    let mut config_path: Option<String> = None;
    let mut verbose = false;
    let mut subcommand: Option<&str> = None;
    let mut init_name: Option<String> = None;
    let mut override_port: Option<u16> = None;
    let mut override_name: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-c" | "--config" => {
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    config_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-p" | "--port" => {
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    if let Ok(port) = args[i + 1].parse::<u16>() {
                        override_port = Some(port);
                    } else {
                        eprintln!("Error: Invalid port number '{}'", args[i + 1]);
                        return Err("Invalid port number".into());
                    }
                    i += 1;
                }
            }
            "-n" | "--name" => {
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    override_name = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-v" | "--verbose" => verbose = true,
            "-h" | "--help" | "help" => {
                print_usage();
                return Ok(());
            }
            "init" => {
                subcommand = Some("init");
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    init_name = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "run" => subcommand = Some("run"),
            _ => {
                // Unknown argument - check if it's a subcommand we don't recognize
                if subcommand.is_none() && !arg.starts_with('-') {
                    eprintln!("Error: Unknown agent subcommand '{arg}'");
                    print_usage();
                    return Err(format!("Unknown subcommand: {arg}").into());
                }
            }
        }
        i += 1;
    }

    // Handle subcommands
    match subcommand {
        Some("init") => {
            if let Some(name) = init_name {
                init_agent(&name)
            } else {
                eprintln!("Error: Agent name required");
                eprintln!("Usage: arkavo agent init <agent-name>");
                Err("Missing agent name".into())
            }
        }
        Some("run") | None => {
            // Run agent with optional config and verbose flag
            run_agent_with_options(
                config_path.as_deref(),
                verbose,
                override_port,
                override_name,
            )
        }
        _ => unreachable!(),
    }
}

fn print_usage() {
    println!("Arkavo Agent - Configure and run AI agents");
    println!();
    println!("USAGE:");
    println!("    arkavo agent [OPTIONS]");
    println!("    arkavo agent init <name>");
    println!();
    println!("SUBCOMMANDS:");
    println!("    init <name>         Create a new .arkavo/AGENTS.md configuration file");
    println!("    run                 Run an agent (alias for default behavior)");
    println!("    help                Print this help message");
    println!();
    println!("OPTIONS:");
    println!(
        "    -c, --config <FILE> Specify config file (default: .arkavo/AGENTS.md or AGENTS.md)"
    );
    println!("    -p, --port <PORT>   Override the listen port (default: random available port)");
    println!("    -n, --name <NAME>   Override the agent name");
    println!("    -v, --verbose       Show startup messages and status");
    println!();
    println!("EXAMPLES:");
    println!("    arkavo agent                           # Run with auto-discovery");
    println!("    arkavo agent --config AGENTS.md        # Run with specific config");
    println!("    arkavo agent --port 8343 -v            # Run on specific port with verbose");
    println!("    arkavo agent -n my-agent -p 8343       # Run with custom name and port");
}

// Extract agent role/purpose from AGENTS.md for use in chat mode
pub fn extract_agent_role() -> Option<String> {
    // Try .arkavo/AGENTS.md first, then fallback to root AGENTS.md
    let config_path = if Path::new(".arkavo/AGENTS.md").exists() {
        ".arkavo/AGENTS.md"
    } else {
        "AGENTS.md"
    };

    if let Ok(content) = fs::read_to_string(config_path)
        && let Ok(agents) = parse_agents_config(&content)
        && let Some(first_agent) = agents.first()
    {
        return Some(first_agent.purpose.clone());
    }
    None
}

fn init_agent(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Create .arkavo directory if it doesn't exist
    let arkavo_dir = Path::new(".arkavo");
    if !arkavo_dir.exists() {
        fs::create_dir_all(arkavo_dir)?;
    }

    let agents_path = arkavo_dir.join("AGENTS.md");

    if agents_path.exists() {
        return Err(".arkavo/AGENTS.md already exists. Please rename or remove it first.".into());
    }

    println!("Creating .arkavo/AGENTS.md configuration for agent '{name}'...");

    // Create a comprehensive template that matches the expected format
    let template = format!(
        r#"# AGENTS.md — {name}

## Agent Identity

- **Name:** {name}
- **Mission:** "An AI agent that assists with specific tasks and capabilities"

## Runtime Configuration

```yaml
listen: 0.0.0.0:8342
mdns: true
```

## Capabilities

Define what this agent can do:

- [ ] Code review and analysis
- [ ] Documentation generation
- [ ] Test creation
- [ ] Bug fixing
- [ ] Performance optimization
- [ ] Data analysis
- [ ] System design
- [ ] Security analysis

## Tool Requirements

Specify which tools this agent needs:

- [ ] Git tools (version control operations)
- [ ] Filesystem access (read/write files)
- [ ] Terminal/Shell (execute commands)
- [ ] Code analysis tools
- [ ] Database access
- [ ] Web APIs
- [ ] Docker/Container management

## MCP Servers

Configure MCP servers that this agent should use:

```yaml
mcp_servers:
  - name: filesystem
    command: mcp-filesystem
    args: []
  - name: git
    command: mcp-git
    args: []
```

## Agent Configuration

Customize the following values for your agent:

```yaml
purpose: "Describe your agent's primary purpose and goals here"
listen: 0.0.0.0:8342
mdns: true
```

## API Keys (Optional)

If your agent needs to access external services, add API keys here:

```yaml
# OPENAI_API_KEY: sk-xxx
# MOONSHOT_API_KEY: sk-xxx
# DEEPSEEK_API_KEY: sk-xxx
```

## Notes

1. Edit the **Mission** field to describe what your agent does
2. Check the capabilities your agent should have
3. Select the tools your agent needs access to
4. Configure MCP servers for additional functionality
5. Update the model if you want to use a different LLM
6. Change the listen address if needed (default: 0.0.0.0:8342)
7. Set mdns to false if you don't want network discovery

## Running Your Agent

Once configured, run your agent with:

```bash
arkavo agent
```

Or specify the config file explicitly:

```bash
arkavo agent --config AGENTS.md
```

Your agent will start and be available at the configured address."#
    );

    fs::write(&agents_path, template)?;
    println!("✓ Created .arkavo/AGENTS.md template for agent '{name}'");
    println!();
    println!("Next steps:");
    println!("1. Edit .arkavo/AGENTS.md to customize your agent's purpose and capabilities");
    println!("2. Configure the model and listen address as needed");
    println!("3. Add any required API keys");
    println!("4. Run your agent with: arkavo agent");

    Ok(())
}

#[allow(clippy::disallowed_methods)]
fn run_agent_with_options(
    config_file: Option<&str>,
    verbose: bool,
    override_port: Option<u16>,
    override_name: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::commands::agent;

    // Determine config path: explicit arg > .arkavo/AGENTS.md > AGENTS.md
    let config_path = if let Some(explicit_path) = config_file {
        Path::new(explicit_path).to_path_buf()
    } else if Path::new(".arkavo/AGENTS.md").exists() {
        Path::new(".arkavo/AGENTS.md").to_path_buf()
    } else {
        Path::new("AGENTS.md").to_path_buf()
    };

    // If AGENTS.md doesn't exist, use default configuration
    let agents = if !config_path.exists() && config_file.is_none() {
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

        let agent_name = format!("{hostname}-{folder_name}");

        vec![AgentConfig {
            name: agent_name,
            purpose: "A general-purpose AI agent".to_string(),
            model: String::new(), // Empty model - let arkavo-router decide
            listen: "0.0.0.0:0".to_string(), // Dynamic port - OS assigns available port
            mdns_enabled: true,
            mcp_servers: Vec::new(),
            api_keys: std::collections::HashMap::new(),
            quiet: true,
            peers: Vec::new(),
            a2a_enabled: true,
            a2a_service_type: None,
        }]
    } else {
        let config_content = fs::read_to_string(&config_path)?;
        match agent::parse_agents_config(&config_content) {
            Ok(agents) if !agents.is_empty() => agents,
            _ => {
                if verbose {
                    eprintln!(
                        "Warning: Could not parse {}, using default configuration",
                        config_path.display()
                    );
                }

                // Fall back to default config
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

                let agent_name = format!("{hostname}-{folder_name}");

                vec![AgentConfig {
                    name: agent_name,
                    purpose: "A general-purpose AI agent".to_string(),
                    model: String::new(),
                    listen: "0.0.0.0:0".to_string(), // Dynamic port
                    mdns_enabled: true,
                    mcp_servers: Vec::new(),
                    api_keys: std::collections::HashMap::new(),
                    quiet: true,
                    peers: Vec::new(),
                    a2a_enabled: true,
                    a2a_service_type: None,
                }]
            }
        }
    };

    // Run the first agent (or fall back to single agent if only one)
    let mut agent_config = agents
        .into_iter()
        .next()
        .ok_or("No agent configuration available")?;

    // Apply command-line overrides
    if let Some(port) = override_port {
        // Parse the current listen address to get the host part
        let host = agent_config.listen.split(':').next().unwrap_or("0.0.0.0");
        agent_config.listen = format!("{host}:{port}");
    }
    if let Some(name) = override_name {
        agent_config.name = name;
    }

    // Set verbose mode - default is quiet (verbose = false)
    agent_config.quiet = !verbose;

    if verbose {
        println!("Starting agent: {}", agent_config.name);
    }

    // Start the A2A server with the agent configuration
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(async { agent::start_agent_server(&agent_config).await }),
        Err(_) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async { agent::start_agent_server(&agent_config).await })
        }
    };

    // If we get an "Invalid listen address" error, fall back to default config
    if let Err(e) = result {
        let err_msg = e.to_string();
        if err_msg.contains("Invalid listen address") {
            if verbose {
                eprintln!("Warning: {err_msg}, using default configuration");
            }

            // Create default config
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

            let default_config = AgentConfig {
                name: format!("{hostname}-{folder_name}"),
                purpose: "A general-purpose AI agent".to_string(),
                model: String::new(),
                listen: "0.0.0.0:0".to_string(), // Dynamic port
                mdns_enabled: true,
                mcp_servers: Vec::new(),
                api_keys: std::collections::HashMap::new(),
                quiet: !verbose,
                peers: Vec::new(),
                a2a_enabled: true,
                a2a_service_type: None,
            };

            if verbose {
                println!("Starting default agent: {}", default_config.name);
            }

            // Try again with default config
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    handle.block_on(async { agent::start_agent_server(&default_config).await })
                }
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new()?;
                    runtime.block_on(async { agent::start_agent_server(&default_config).await })
                }
            }
        } else {
            Err(e)
        }
    } else {
        result
    }
}

// Agent configuration parsing
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub purpose: String, // Used as system prompt for LLM
    pub model: String,
    pub listen: String,
    pub mdns_enabled: bool,
    pub mcp_servers: Vec<McpServerConfig>,
    pub api_keys: std::collections::HashMap<String, String>,
    pub quiet: bool, // Default true (quiet), false if --verbose is specified
    // A2A peer configuration
    pub peers: Vec<String>,               // e.g., ["http://localhost:8352"]
    pub a2a_enabled: bool,                // Default: true
    pub a2a_service_type: Option<String>, // Custom mDNS service type
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
    let mut in_peers_section = false;
    let mut in_args_section = false;
    let mut in_a2a_section = false;
    let mut in_purpose_multiline = false;
    let mut purpose_lines: Vec<String> = Vec::new();

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
                    model: String::new(),
                    listen: "0.0.0.0:0".to_string(),
                    mdns_enabled: true,
                    mcp_servers: Vec::new(),
                    api_keys: std::collections::HashMap::new(),
                    quiet: true, // Default is quiet
                    peers: Vec::new(),
                    a2a_enabled: true,
                    a2a_service_type: None,
                });
                in_agent_section = true;
                continue;
            }

            // Track current markdown section
            if trimmed.starts_with("## ") {
                let section_name = trimmed.strip_prefix("## ").unwrap_or("").to_string();

                // Check if this is an agent definition (not a standard section)
                let is_standard_section = section_name.starts_with("Agent Identity")
                    || section_name.starts_with("Runtime Configuration")
                    || section_name.starts_with("Capabilities")
                    || section_name.starts_with("Tool Requirements")
                    || section_name.starts_with("MCP Server")
                    || section_name.starts_with("Purpose")
                    || section_name.starts_with("Model Configuration")
                    || section_name.starts_with("Rover Configuration")
                    || section_name.starts_with("A2A Protocol")
                    || section_name.starts_with("Logging");

                if !is_standard_section {
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
                        model: String::new(),
                        listen: "0.0.0.0:0".to_string(),
                        mdns_enabled: true,
                        mcp_servers: Vec::new(),
                        api_keys: std::collections::HashMap::new(),
                        quiet: true, // Default is quiet
                        peers: Vec::new(),
                        a2a_enabled: true,
                        a2a_service_type: None,
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
                            else if trimmed.starts_with("- **Mission:**")
                                && let Some(mission) =
                                    extract_markdown_field_value(trimmed, "**Mission:**")
                            {
                                agent.purpose = mission;
                            }
                        }
                        "Runtime Configuration (example)" => {
                            // Try to extract listen address from YAML block
                            if trimmed.starts_with("listen:")
                                && let Some(listen) = extract_yaml_value(trimmed, "listen:")
                            {
                                agent.listen = listen;
                            }
                        }
                        _ => {}
                    }
                }

                // Also handle direct YAML-style properties in markdown (for flexibility)
                parse_yaml_properties(
                    line,
                    trimmed,
                    agent,
                    &mut in_mcp_section,
                    &mut current_mcp_server,
                    &mut in_peers_section,
                    &mut in_a2a_section,
                    &mut in_args_section,
                    &mut in_purpose_multiline,
                    &mut purpose_lines,
                );
            }
        } else {
            // Handle old YAML-style format
            // Check for single # header (creates new agent)
            let is_top_level_header = trimmed.starts_with("# ") && !trimmed.starts_with("## ");

            // Check for ## header that's not a standard section
            let is_new_agent_section = if trimmed.starts_with("## ") {
                let section_name = trimmed.strip_prefix("## ").unwrap_or("").trim();
                // Standard sections don't create new agents
                !section_name.starts_with("Agent Identity")
                    && !section_name.starts_with("Runtime Configuration")
                    && !section_name.starts_with("Capabilities")
                    && !section_name.starts_with("Tool Requirements")
                    && !section_name.starts_with("MCP Server")
                    && !section_name.starts_with("Purpose")
                    && !section_name.starts_with("Model Configuration")
                    && !section_name.starts_with("Rover Configuration")
                    && !section_name.starts_with("A2A Protocol")
                    && !section_name.starts_with("Logging")
            } else {
                false
            };

            let is_agent_header = is_top_level_header || is_new_agent_section;

            if is_agent_header {
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

                // Extract name from header
                let header_prefix = if trimmed.starts_with("## ") {
                    "## "
                } else {
                    "# "
                };
                let header_text = trimmed.strip_prefix(header_prefix).unwrap_or("").trim();

                // Use a default name that will be overridden by explicit name: field
                let name = header_text.to_string();

                current_agent = Some(AgentConfig {
                    name,
                    purpose: String::new(),
                    model: String::new(),
                    listen: "0.0.0.0:0".to_string(), // Dynamic port
                    mdns_enabled: true,              // Default to true for zero-config
                    mcp_servers: Vec::new(),
                    api_keys: std::collections::HashMap::new(),
                    quiet: true, // Default is quiet
                    peers: Vec::new(),
                    a2a_enabled: true,
                    a2a_service_type: None,
                });
                in_agent_section = true;
                continue;
            }

            // Skip if not in agent section
            if !in_agent_section || current_agent.is_none() {
                continue;
            }

            if let Some(agent) = current_agent.as_mut() {
                parse_yaml_properties(
                    line,
                    trimmed,
                    agent,
                    &mut in_mcp_section,
                    &mut current_mcp_server,
                    &mut in_peers_section,
                    &mut in_a2a_section,
                    &mut in_args_section,
                    &mut in_purpose_multiline,
                    &mut purpose_lines,
                );
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

/// Check if a tool's input schema has required arguments
fn has_required_args(schema: &serde_json::Value) -> bool {
    // Check if there's a "required" array with any entries
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .is_some_and(|arr| !arr.is_empty())
}

// Helper function to parse YAML-style properties (used by both old and new format parsers)
#[allow(clippy::too_many_arguments)]
fn parse_yaml_properties(
    line: &str,
    trimmed: &str,
    agent: &mut AgentConfig,
    in_mcp_section: &mut bool,
    current_mcp_server: &mut Option<McpServerConfig>,
    in_peers_section: &mut bool,
    in_a2a_section: &mut bool,
    in_args_section: &mut bool,
    in_purpose_multiline: &mut bool,
    purpose_lines: &mut Vec<String>,
) {
    // Check for a2a section
    if trimmed == "a2a:" {
        *in_a2a_section = true;
        return;
    }

    // Check for peers section (can be top-level or under a2a)
    if trimmed == "peers:" {
        *in_peers_section = true;
        return;
    }

    // Handle peer entries
    if *in_peers_section && trimmed.starts_with("- ") {
        let peer = trimmed
            .strip_prefix("- ")
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_string();
        if !peer.is_empty() {
            agent.peers.push(peer);
        }
        return;
    }

    // End peers section when we hit a non-list item
    if *in_peers_section
        && !trimmed.is_empty()
        && !trimmed.starts_with('-')
        && !trimmed.starts_with(' ')
    {
        *in_peers_section = false;
    }

    // Parse a2a properties
    if *in_a2a_section && !*in_peers_section {
        if trimmed.starts_with("enabled:") {
            agent.a2a_enabled = !trimmed.contains("false");
            return;
        } else if trimmed.starts_with("service_type:") {
            agent.a2a_service_type = Some(
                trimmed
                    .strip_prefix("service_type:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
            return;
        }
        // End a2a section when we hit a non-indented, non-a2a property
        if !trimmed.is_empty()
            && !trimmed.starts_with(' ')
            && !trimmed.starts_with('-')
            && !trimmed.starts_with("enabled:")
            && !trimmed.starts_with("service_type:")
            && trimmed != "peers:"
            && !trimmed.starts_with("discovery:")
        {
            *in_a2a_section = false;
        }
    }

    // Check for mcp_servers section
    if trimmed == "mcp_servers:" {
        *in_mcp_section = true;
        *in_a2a_section = false;
        *in_peers_section = false;
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

    // Handle YAML list args (e.g., "- value")
    if *in_args_section && trimmed.starts_with("- ") {
        if let Some(server) = current_mcp_server.as_mut() {
            let arg = trimmed
                .strip_prefix("- ")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
            if !arg.is_empty() {
                server.args.push(arg);
            }
        }
        return;
    }

    // End args section when we hit a non-list item
    if *in_args_section && !trimmed.starts_with("- ") && !trimmed.is_empty() {
        *in_args_section = false;
    }

    // Parse MCP server properties
    if *in_mcp_section && let Some(server) = current_mcp_server.as_mut() {
        if trimmed.starts_with("command:") {
            *in_args_section = false;
            server.command = Some(
                trimmed
                    .strip_prefix("command:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
        } else if trimmed.starts_with("args:") {
            // Check for inline array format: ["arg1", "arg2"]
            let args_str = trimmed.strip_prefix("args:").unwrap_or("").trim();
            if args_str.starts_with('[') && args_str.ends_with(']') {
                let args_content = &args_str[1..args_str.len() - 1];
                server.args = args_content
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            } else if args_str.is_empty() {
                // Start YAML list mode for args
                *in_args_section = true;
            }
        } else if trimmed.starts_with("url:") {
            *in_args_section = false;
            server.url = Some(
                trimmed
                    .strip_prefix("url:")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
        } else if trimmed.starts_with("transport:") {
            *in_args_section = false;
            // Skip transport for now, not used
        } else if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('-') {
            // End of MCP section
            *in_mcp_section = false;
            *in_args_section = false;
            if let Some(server) = current_mcp_server.take() {
                agent.mcp_servers.push(server);
            }
        }
    }

    // Parse agent properties (when not in MCP section)
    if !*in_mcp_section {
        if trimmed.starts_with("name:") && !trimmed.starts_with("name: |") {
            agent.name = trimmed
                .strip_prefix("name:")
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
        } else if trimmed.starts_with("purpose:") {
            let value = trimmed.strip_prefix("purpose:").unwrap_or("").trim();
            if value == "|" || value == "|-" || value == "|+" {
                // Start multi-line YAML string
                *in_purpose_multiline = true;
                purpose_lines.clear();
            } else {
                // Single-line purpose
                agent.purpose = value.trim_matches('"').to_string();
            }
        } else if *in_purpose_multiline {
            // Collect multi-line purpose content
            if line.starts_with("  ") || line.starts_with('\t') {
                // Indented line - part of multi-line value
                purpose_lines.push(line.trim().to_string());
            } else if trimmed.is_empty() {
                // Empty line in multi-line - preserve as paragraph break
                purpose_lines.push(String::new());
            } else {
                // Non-indented, non-empty line - end of multi-line
                *in_purpose_multiline = false;
                agent.purpose = purpose_lines.join("\n").trim().to_string();
                purpose_lines.clear();
                // Re-process this line (it might be a new property)
                // Continue to let other parsers handle it
            }
        }

        // End multi-line purpose on new property/section
        if *in_purpose_multiline && (trimmed.starts_with("model:") || trimmed.starts_with('#')) {
            *in_purpose_multiline = false;
            agent.purpose = purpose_lines.join("\n").trim().to_string();
            purpose_lines.clear();
        }

        if trimmed.starts_with("model:") {
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
        // Note: autonomous_interval and event_tool are deprecated - push notifications are used instead
    }
}

#[allow(clippy::future_not_send)]
#[allow(clippy::missing_panics_doc)]
pub async fn start_agent_server(config: &AgentConfig) -> Result<(), Box<dyn std::error::Error>> {
    use crate::mcp_spawner::McpProcessManager;
    use arkavo_crypto::AgentKeypair;
    use arkavo_gossip::GossipConfig;
    use arkavo_protocol::{config::ServerConfig, rate_limit::RateLimitConfig};
    use arkavo_server::A2aServer;
    use arkavo_server::{LearningBus, start_anti_entropy_loop, start_lesson_propagation_loop};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    // Create AgentIdentity for TDF encryption/decryption
    let mut identity_attributes = HashMap::new();
    identity_attributes.insert("agent.id".to_string(), config.name.clone());
    let agent_identity = Arc::new(
        AgentIdentity::new(config.name.clone(), identity_attributes)
            .map_err(|e| format!("Failed to create AgentIdentity: {e}"))?,
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

    // Set the public key for TDF encryption (used in agent.capabilities.get RPC)
    server.set_public_key(public_key_b64.clone()).await;

    // Initialize LearningBus FIRST (before set_agent_metadata which initializes router)
    let learning_bus = {
        let keypair = Arc::new(AgentKeypair::generate());
        let gossip_config = GossipConfig::default();
        Arc::new(LearningBus::new(
            config.name.clone(),
            "default-swarm".to_string(),
            keypair,
            gossip_config,
        ))
    };
    server.set_learning_bus(learning_bus.clone()).await;

    // Set agent metadata (this initializes the router which will be set on learning bus)
    server
        .set_agent_metadata(
            config.name.clone(),
            config.purpose.clone(),
            config.model.clone(),
        )
        .await;

    // Set API keys in the server
    server.set_api_keys(config.api_keys.clone()).await;

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
            // Create external MCP connection
            use crate::mcp_integration::McpConnection;
            match McpConnection::new_external(Some(url.clone())) {
                Ok(connection) => {
                    let wrapped = McpConnectionWrapper::new(connection);
                    mcp_registry
                        .register(mcp_config.name.clone(), Box::new(wrapped))
                        .await;

                    if debug_mode {
                        println!(
                            "[MCP] External server connected: name={} url={}",
                            mcp_config.name, url
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

    let (handle, actual_port) = server.start_with_port().await?;

    // Start orchestrator loop if this agent has MCP tools configured
    if !config.mcp_servers.is_empty() {
        server.start_orchestrator_loop().await;
    }

    // Generate and display QR code for registration
    if !quiet {
        use arkavo_crypto::AgentKeypair;
        use arkavo_device_identity::{get_or_create_device_id, keypair};
        use arkavo_registration::{AgentDescriptor, qr::display_authorization_qr};

        // Get or create device ID (needed for system initialization)
        let _device_id =
            get_or_create_device_id().map_err(|e| format!("Failed to get device ID: {e}"))?;

        // Get or create agent keypair
        let keypair_bytes = match keypair::get_keypair()? {
            Some(bytes) => bytes,
            None => {
                let new_keypair = AgentKeypair::generate();
                let bytes = new_keypair.to_bytes();
                keypair::store_keypair(&bytes)?;
                bytes
            }
        };

        let agent_keypair =
            AgentKeypair::from_bytes(&keypair_bytes).expect("Invalid keypair bytes");
        let public_key = agent_keypair.public_key();

        // Extract folder name (last part of agent name) for display
        let folder_id = config
            .name
            .rsplit('-')
            .next()
            .unwrap_or("unknown")
            .to_string();

        // Create agent descriptor with DID:key and default entitlements
        // Use actual local IP and bound port (not 0.0.0.0:0 from config)
        let local_ip = get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
        let endpoint = format!("http://{local_ip}:{actual_port}");
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
                actual_port,
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
        let learning_bus_transport = learning_bus.clone();
        gossip_handles.push(tokio::spawn(async move {
            use arkavo_protocol::http::HttpTransport;
            use arkavo_protocol::transport::{
                A2aEndpoint, A2aRequest, A2aTransport, TransportConfig,
            };

            let mut rx = learning_bus_transport.subscribe_gossip_out();
            loop {
                match rx.recv().await {
                    Ok((peer_id, message)) => {
                        // Look up peer address
                        if let Some(addr) = learning_bus_transport.get_peer_address(&peer_id).await
                        {
                            // Create JSON-RPC request for gossip/message
                            // jsonrpsee expects params as {"message": <value>}
                            let params = serde_json::json!({
                                "message": message
                            });
                            let request = A2aRequest::new("gossip/message", params);

                            // Create transport and send (allow non-TLS for local mDNS discovery)
                            let mut config = TransportConfig::default();
                            config.tls_config.require_tls = false;
                            let transport = match HttpTransport::new(config) {
                                Ok(t) => t,
                                Err(e) => {
                                    tracing::warn!("Failed to create transport: {}", e);
                                    continue;
                                }
                            };

                            let endpoint = A2aEndpoint {
                                url: addr.clone(),
                                agent_id: peer_id.clone(),
                                public_key: None,
                            };
                            if let Err(e) = transport.connect(&endpoint).await {
                                tracing::warn!("Failed to connect to {}: {}", peer_id, e);
                                continue;
                            }

                            match transport.send_request(request).await {
                                Ok(_response) => {
                                    tracing::debug!("Gossip sent to {}", peer_id);
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to send gossip to {}: {}", peer_id, e);
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
        // Display the actual bound address (using actual_port from OS if port was 0)
        let listen_host = config.listen.split(':').next().unwrap_or("0.0.0.0");
        println!("Ready at {listen_host}:{actual_port}");
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

    // Small delay to ensure GPU cleanup completes
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Explicitly exit to ensure all threads are terminated
    std::process::exit(0);
}

/// Get the local IP address for client connections.
///
/// Uses multiple strategies to determine the local IP:
/// 1. Try connecting to a public DNS server (determines routing interface)
/// 2. Fallback to interface enumeration
/// 3. Final fallback to 127.0.0.1
///
/// This handles offline environments, strict firewalls, and IPv6-only networks.
fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;

    // Strategy 1: DNS-based detection (works in most online environments)
    // Try multiple public DNS servers for resilience
    let dns_servers = [
        ("8.8.8.8", 80),        // Google DNS
        ("1.1.1.1", 80),        // Cloudflare DNS
        ("208.67.222.222", 80), // OpenDNS
    ];

    for (dns, port) in dns_servers {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0")
            && socket.connect((dns, port)).is_ok()
            && let Ok(local_addr) = socket.local_addr()
        {
            let ip = local_addr.ip();
            if !ip.is_loopback() && ip.is_ipv4() {
                return Some(ip.to_string());
            }
        }
    }

    // Strategy 2: Try connecting to a private network address
    // This works in offline LAN environments
    let private_targets = [
        ("192.168.1.1", 53), // Common router address
        ("10.0.0.1", 53),    // Common corporate router
        ("172.16.0.1", 53),  // Common large network router
    ];

    for (target, port) in private_targets {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0")
            && socket.connect((target, port)).is_ok()
            && let Ok(local_addr) = socket.local_addr()
        {
            let ip = local_addr.ip();
            if !ip.is_loopback() && ip.is_ipv4() {
                return Some(ip.to_string());
            }
        }
    }

    // Strategy 3: Fallback to 127.0.0.1 (always works, but only for local clients)
    // This is the safest fallback for offline or isolated environments
    Some("127.0.0.1".to_string())
}

fn broadcast_agent_mdns_sync(
    #[allow(unused_variables)] config: &AgentConfig,
    #[allow(unused_variables)] actual_port: u16,
    #[allow(unused_variables)] shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[allow(unused_variables)] ready_signal: Option<std::sync::mpsc::Sender<()>>,
    #[allow(unused_variables)] peer_tx: tokio::sync::mpsc::Sender<(String, bool, Option<String>)>,
    #[allow(unused_variables)] public_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mdns")]
    {
        use mdns_sd::{ServiceDaemon, ServiceInfo};
        use std::collections::HashMap;
        use std::thread;
        use std::time::Duration;

        let port = actual_port;
        let service_ip = get_service_ip();

        // Create mDNS daemon
        let mdns = ServiceDaemon::new()?;

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

        // Prepare properties - minimal mDNS, full capabilities via RPC
        let mut properties = HashMap::new();
        properties.insert("agent_id".to_string(), config.name.clone());
        properties.insert("ip".to_string(), service_ip.to_string());
        properties.insert("version".to_string(), "1.0".to_string());

        // Add public key for TDF encryption (agents broadcast their ECDSA P-256 public key)
        if let Some(pk) = &public_key {
            properties.insert("public_key".to_string(), pk.clone());
        }

        // Retain purpose and model for backward compatibility with existing orchestrators
        // Full capability details should be queried via agent.capabilities.get RPC
        if !config.purpose.is_empty() {
            properties.insert("purpose".to_string(), config.purpose.clone());
        }
        if !config.model.is_empty() {
            properties.insert("model".to_string(), config.model.clone());
        }

        // Add capabilities based on agent name/purpose (for backward compatibility)
        let capabilities = get_agent_capabilities(&config.name, &config.purpose);
        if !capabilities.is_empty() {
            properties.insert("capabilities".to_string(), capabilities.join(","));
        }

        // Add MCP servers as capabilities (for backward compatibility)
        if !config.mcp_servers.is_empty() {
            let mcp_tools: Vec<String> =
                config.mcp_servers.iter().map(|s| s.name.clone()).collect();
            properties.insert("mcp_tools".to_string(), mcp_tools.join(","));
        }

        // Create service info
        let service_type = "_a2a._tcp.local.";
        let instance_name = config.name.clone();
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
