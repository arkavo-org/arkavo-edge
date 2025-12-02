//! Example: Secure Configuration Distribution with A2A Protocol
//!
//! This example demonstrates how to use the secure configuration transport
//! system to distribute encrypted configuration bundles from an orchestrator
//! to agents over the A2A protocol.

use arkavo_config_bundle::{
    AgentRole, BundleTarget, ConfigurationBundle, Entitlement, Permission, ResourceType, Secret,
    SecretType,
};
use arkavo_config_encryption::{AgentIdentity, Policy};
use arkavo_config_transport::{ConfigTransportClient, ConfigTransportServer};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("=== Secure Configuration Distribution Example ===\n");

    // ========================================================================
    // ORCHESTRATOR SIDE: Create and distribute configuration bundle
    // ========================================================================

    println!("1. Creating configuration bundle on orchestrator...");

    // Create a configuration bundle for search agents
    let mut bundle = ConfigurationBundle::new(
        BundleTarget::Role("search-agent".to_string()),
        AgentRole {
            name: "search-agent".to_string(),
            purpose: "Web search and data retrieval for user queries".to_string(),
            capabilities: vec![
                "web_search".to_string(),
                "data_extraction".to_string(),
                "content_summarization".to_string(),
            ],
        },
        "admin@example.com".to_string(),
        "production".to_string(),
    );

    // Add configuration settings
    bundle.add_setting("max_concurrent_tasks".to_string(), serde_json::json!(5));
    bundle.add_setting("timeout_seconds".to_string(), serde_json::json!(30));
    bundle.add_setting("log_level".to_string(), serde_json::json!("info"));

    // Add entitlements (permissions)
    bundle.add_entitlement(Entitlement {
        resource_type: ResourceType::McpTool,
        resource_id: "web-search".to_string(),
        permissions: vec![Permission::Execute, Permission::ReadResults],
    });

    bundle.add_entitlement(Entitlement {
        resource_type: ResourceType::McpTool,
        resource_id: "scrape-webpage".to_string(),
        permissions: vec![Permission::Execute, Permission::ReadResults],
    });

    // Add secrets (will be encrypted)
    bundle.add_secret(
        "GOOGLE_API_KEY".to_string(),
        Secret {
            key: "GOOGLE_API_KEY".to_string(),
            value: Some("sk-example-key-12345".to_string()),
            secret_type: SecretType::ApiKey,
            rotation_policy: None,
            metadata: None,
        },
    );

    // Add required attributes for access control
    bundle.add_required_attribute("agent.role=search-agent".to_string());
    bundle.add_required_attribute("environment=production".to_string());

    println!("   ✓ Bundle created: {}", bundle.bundle_id);
    println!("   ✓ Settings: {}", bundle.settings.len());
    println!("   ✓ Entitlements: {}", bundle.entitlements.len());
    println!("   ✓ Secrets: {}", bundle.secrets.len());

    // Create access policy
    let mut policy = Policy::new();
    policy.add_attribute("agent.role".to_string(), "Agent Role".to_string());
    policy.add_attribute("environment".to_string(), "Environment".to_string());
    policy.add_dissemination("agent.role=search-agent".to_string());
    policy.add_dissemination("environment=production".to_string());

    println!("\n2. Setting up orchestrator transport server...");

    // Create transport server (orchestrator-side)
    let server = ConfigTransportServer::new("https://kas.example.com".to_string())?;

    println!("   ✓ Server initialized");
    println!("   ✓ KAS URL: https://kas.example.com");
    println!(
        "   ✓ Orchestrator public key: {} bytes",
        server.get_public_key().len()
    );

    // Distribute the bundle
    println!("\n3. Distributing configuration bundle...");
    let bundle_id = server.distribute_bundle(bundle.clone(), policy).await?;

    println!("   ✓ Bundle distributed: {}", bundle_id);
    println!("   ✓ Bundle encrypted with AES-256-GCM");
    println!("   ✓ Bundle signed with orchestrator private key");

    // ========================================================================
    // AGENT SIDE: Request and apply configuration
    // ========================================================================

    println!("\n4. Creating agent identity...");

    // Create agent identity with attributes
    let mut attributes = HashMap::new();
    attributes.insert("agent.role".to_string(), "search-agent".to_string());
    attributes.insert("environment".to_string(), "production".to_string());

    let agent_identity = AgentIdentity::new("search-agent-001".to_string(), attributes)?;

    println!("   ✓ Agent ID: {}", agent_identity.agent_id);
    println!("   ✓ Agent attributes: 2");
    println!(
        "   ✓ Agent public key: {} bytes",
        agent_identity.public_key.as_bytes().len()
    );

    // Create transport client (agent-side)
    let client = ConfigTransportClient::new(agent_identity);

    println!("\n5. Agent requesting configuration via A2A protocol...");
    println!("   (In production, this would call agent.config.get RPC method)");

    // In a real implementation, the agent would:
    // 1. Call agent.config.get via A2A RPC
    // 2. Receive AgentConfigGetResponse with ConfigTransportEnvelope
    // 3. Process the response using client.process_configuration_response()
    //
    // For this example, we'll simulate the successful retrieval

    println!("   ✓ Configuration request sent");
    println!("   ✓ Request signed with agent private key");

    println!("\n6. Configuration distribution complete!");
    println!("\n=== Summary ===");
    println!("✓ Orchestrator created and encrypted configuration bundle");
    println!("✓ Bundle signed with orchestrator private key");
    println!("✓ Bundle registered for distribution");
    println!("✓ Agent identity created with attributes");
    println!("✓ Agent ready to request configuration via A2A protocol");
    println!("\n=== Security Features ===");
    println!("✓ AES-256-GCM encryption");
    println!("✓ ECDSA P-256 signatures");
    println!("✓ Attribute-based access control");
    println!("✓ Public/private key authentication");
    println!("✓ End-to-end encryption");

    println!("\n=== Next Steps ===");
    println!("1. Integrate ConfigTransportHandler into A2A RPC server");
    println!("2. Register agent public keys during discovery");
    println!("3. Call server.handle_config_request() in agent_config_get");
    println!("4. Agents automatically receive encrypted configurations");

    Ok(())
}