//! Example demonstrating encryption and decryption with Arkavo KAS
//!
//! This example shows how to:
//! 1. Create a configuration bundle
//! 2. Encrypt it with OpenTDF using attribute-based policies
//! 3. Decrypt it using the Arkavo KAS at https://platform.arkavo.net
//!
//! Usage:
//!   cargo run --example kas_encrypt_decrypt --features kas

use arkavo_config_bundle::{AgentRole, BundleTarget, ConfigurationBundle};
use arkavo_config_encryption::{
    AgentCredential, ConfigBundleDecryptor, ConfigBundleEncryptor, Policy, PolicyAttribute,
};
use std::collections::HashMap;

const KAS_URL: &str = "https://platform.arkavo.net/kas/v2/rewrap";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Arkavo Secure Agent Configuration Example ===\n");

    // Step 1: Create a configuration bundle for an agent
    println!("Step 1: Creating configuration bundle...");
    let mut bundle = ConfigurationBundle::new(
        BundleTarget::Role("monitoring-agent".to_string()),
        AgentRole {
            name: "monitoring-agent".to_string(),
            purpose: "System monitoring and metrics collection".to_string(),
            capabilities: vec![
                "metrics-collection".to_string(),
                "log-aggregation".to_string(),
                "alerting".to_string(),
            ],
        },
        "orchestrator".to_string(),
        "production".to_string(),
    );

    // Add configuration settings
    bundle.add_setting("metrics_interval".to_string(), serde_json::json!(60));
    bundle.add_setting("log_level".to_string(), serde_json::json!("info"));
    bundle.add_setting(
        "endpoints".to_string(),
        serde_json::json!(["https://api.example.com", "https://backup.example.com"]),
    );

    // Add required attributes for ABAC
    bundle.add_required_attribute("agent.role=monitoring-agent".to_string());
    bundle.add_required_attribute("environment=production".to_string());

    println!("  ✓ Bundle ID: {}", bundle.bundle_id);
    println!("  ✓ Role: {}", bundle.role.name);
    println!("  ✓ Settings: {} configured", bundle.settings.len());
    println!();

    // Step 2: Define access policy using Arkavo attribute schema
    println!("Step 2: Defining access policy...");
    let policy = Policy {
        attributes: vec![
            PolicyAttribute {
                attribute: "data/clearance".to_string(),
                value: "internal".to_string(),
                display_name: "Data Clearance".to_string(),
            },
            PolicyAttribute {
                attribute: "agent/identity".to_string(),
                value: "autonomous_service".to_string(),
                display_name: "Agent Identity".to_string(),
            },
        ],
        dissemination: vec![
            "agent.role=monitoring-agent".to_string(),
            "environment=production".to_string(),
        ],
    };

    println!("  ✓ Attributes: {:?}", policy.attributes.len());
    println!("  ✓ Dissemination rules: {:?}", policy.dissemination);
    println!();

    // Step 3: Encrypt bundle with OpenTDF
    println!("Step 3: Encrypting with OpenTDF...");
    println!("  KAS URL: {}", KAS_URL);

    let encryptor = ConfigBundleEncryptor::new(KAS_URL.to_string());
    let encrypted = encryptor.encrypt_bundle(&bundle, policy)?;

    println!("  ✓ Encrypted bundle created");
    println!("  ✓ Policy ID: {}", encrypted.policy_manifest.policy_id);
    println!(
        "  ✓ Encrypted data size: {} bytes",
        encrypted.encrypted_data.len()
    );
    println!("  ✓ Created at: {}", encrypted.created_at);
    println!();

    // Step 4: Create agent identity with matching attributes
    println!("Step 4: Creating agent identity...");
    let mut agent_attributes = HashMap::new();
    agent_attributes.insert("agent.role".to_string(), "monitoring-agent".to_string());
    agent_attributes.insert("environment".to_string(), "production".to_string());

    let agent_identity =
        AgentCredential::new("monitoring-agent-001".to_string(), agent_attributes)?;

    println!("  ✓ Agent ID: {}", agent_identity.agent_id);
    println!("  ✓ Attributes: {:?}", agent_identity.attributes);
    println!("  ✓ Public/Private key pair generated");
    println!();

    // Step 5: Attempt decryption (will require KAS in production)
    println!("Step 5: Decryption status...");
    let decryptor = ConfigBundleDecryptor::new(agent_identity);

    // In production, this would use decrypt_bundle_async with KAS client
    match decryptor.decrypt_bundle(&encrypted) {
        Ok(_) => println!("  ✓ Decryption successful (unexpected in example)"),
        Err(e) => {
            if e.to_string().contains("KAS integration") {
                println!("  ⚠ Synchronous decryption not available");
                println!("  ℹ Use decrypt_bundle_async() with KAS client for production");
                println!();
                println!("Production usage:");
                println!("  ```rust");
                println!("  use opentdf::KasClient;");
                println!();
                println!(
                    "  let kas_client = KasClient::new(\"{}\", oauth_token)?;",
                    KAS_URL
                );
                println!("  let bundle = decryptor");
                println!("      .decrypt_bundle_async(&encrypted, kas_client)");
                println!("      .await?;");
                println!("  ```");
            } else {
                println!("  ✗ Decryption failed: {}", e);
            }
        }
    }
    println!();

    // Summary
    println!("=== Summary ===");
    println!();
    println!("This example demonstrated:");
    println!("  ✓ Configuration bundle creation with settings and roles");
    println!("  ✓ Attribute-based access control (ABAC) policy definition");
    println!("  ✓ OpenTDF encryption with cryptographic policy binding");
    println!("  ✓ Agent identity with public/private key pair");
    println!("  ✓ KAS URL configuration for key unwrapping");
    println!();
    println!("Next steps for production:");
    println!("  • Enable 'kas' feature flag");
    println!("  • Configure OAuth token for KAS authentication");
    println!("  • Use async decrypt_bundle_async() method");
    println!("  • Implement proper error handling and retry logic");
    println!("  • Add audit logging for compliance");
    println!();
    println!("KAS Endpoint: {}", KAS_URL);

    Ok(())
}
