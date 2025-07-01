use anyhow::Result;
use std::process::Command;
use tracing::{info, warn};

pub async fn run_a2a_demo(agent1_port: u16, agent2_port: u16, use_websocket: bool) -> Result<()> {
    info!("Starting A2A Transport Demo");
    info!("Agent 1 port: {}", agent1_port);
    info!("Agent 2 port: {}", agent2_port);
    info!(
        "Transport: {}",
        if use_websocket { "WebSocket" } else { "HTTP" }
    );

    // First, ensure the library is built
    info!("Building arkavo-protocol library...");
    let output = Command::new("cargo")
        .args(&["build", "-p", "arkavo-protocol", "--lib"])
        .output()?;

    if !output.status.success() {
        warn!("Failed to build arkavo-protocol:");
        warn!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(anyhow::anyhow!("Build failed"));
    }

    info!("Library built successfully!");

    // Run the API demonstration
    demonstrate_api_usage(agent1_port, agent2_port, use_websocket).await?;

    Ok(())
}

async fn demonstrate_api_usage(
    agent1_port: u16,
    agent2_port: u16,
    use_websocket: bool,
) -> Result<()> {
    info!("");
    info!("=== A2A Transport Layer API Demonstration ===");
    info!("");
    info!("The A2A Transport Layer provides:");
    info!("1. Transport abstraction (HTTP/WebSocket)");
    info!("2. Service discovery mechanisms");
    info!("3. Security configuration (TLS, mTLS)");
    info!("4. Automatic retry and error handling");
    info!("");
    info!("Example usage:");
    info!("");
    info!("// Create transport");
    info!(
        "let transport = {}Transport::new(TransportConfig::default());",
        if use_websocket { "WebSocket" } else { "Http" }
    );
    info!("");
    info!("// Define endpoints");
    info!("let agent1 = A2aEndpoint {{");
    info!(
        "    url: \"{}://localhost:{}\".to_string(),",
        if use_websocket { "ws" } else { "http" },
        agent1_port
    );
    info!("    agent_id: \"agent-1\".to_string(),");
    info!("    public_key: None,");
    info!("}};");
    info!("");
    info!("let agent2 = A2aEndpoint {{");
    info!(
        "    url: \"{}://localhost:{}\".to_string(),",
        if use_websocket { "ws" } else { "http" },
        agent2_port
    );
    info!("    agent_id: \"agent-2\".to_string(),");
    info!("    public_key: None,");
    info!("}};");
    info!("");
    info!("// Service Discovery");
    info!("let discovery = DiscoveryService::new(DiscoveryConfig::default());");
    info!("discovery.register_static_endpoint(agent1);");
    info!("discovery.register_static_endpoint(agent2);");
    info!("");
    info!("// Connect to agent");
    info!("let endpoint = discovery.discover_agent(\"agent-1\")?;");
    info!("transport.connect(&endpoint).await?;");
    info!("");
    info!("// Send request");
    info!("let request = A2aRequest::new(");
    info!("    \"method.name\",");
    info!("    serde_json::json!({{\"key\": \"value\"}})");
    info!(");");
    info!("let response = transport.send_request(request).await?;");
    info!("");
    info!("// Close connection");
    info!("transport.close().await?;");
    info!("");
    info!("=== Configuration Options ===");
    info!("");
    info!("Transport Configuration:");
    info!("- Timeout: 30000ms");
    info!("- Max retries: 3");
    info!("- Retry delay: 1000ms");
    info!("- TLS required: true");
    info!("");
    info!("Security Configuration:");
    info!("- TLS enabled by default");
    info!("- Certificate verification: true");
    info!("- Minimum TLS version: 1.2");
    info!("- Optional mutual TLS support");
    info!("");
    info!("Discovery Methods:");
    info!("- Static registration (shown above)");
    info!("- Environment variables: A2A_AGENT_<NAME>_URL");
    info!("- mDNS support (planned)");
    info!("- DNS SRV records (planned)");
    info!("");
    info!("=== Next Steps ===");
    info!("");
    info!("To create a working agent:");
    info!("");
    info!("1. Implement a JSON-RPC server using jsonrpsee:");
    info!("   - Use jsonrpsee::server::Server");
    info!("   - Register methods to handle requests");
    info!("   - Start server on desired port");
    info!("");
    info!("2. Create agent logic:");
    info!("   - Define promise-related methods");
    info!("   - Handle incoming requests");
    info!("   - Send responses back to clients");
    info!("");
    info!("3. Test with multiple agents:");
    info!("   - Start multiple server instances");
    info!("   - Use discovery to find agents");
    info!("   - Exchange messages between agents");
    info!("");
    info!("Demo completed!");

    Ok(())
}
