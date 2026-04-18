//! Example showing OAuth authentication flow with Arkavo Identity
//!
//! This example demonstrates how to authenticate with the Arkavo Identity service
//! to obtain tokens for KAS operations.
//!
//! Usage:
//!   cargo run --example oauth_authentication

use arkavo_config_encryption::{DEFAULT_IDENTITY_URL, DEFAULT_KAS_URL, KasConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Arkavo OAuth Authentication Example ===\n");

    // Step 1: Configure endpoints
    println!("Step 1: Arkavo Service Endpoints");
    let kas_config = KasConfig::production();

    println!("  KAS Endpoint:      {}", kas_config.rewrap_url());
    println!("  Identity Endpoint: {}", kas_config.identity_url());
    println!();

    // Step 2: Show environment variable configuration
    println!("Step 2: Environment Variable Configuration");
    println!("  You can override defaults using:");
    println!("    export ARKAVO_KAS_URL=\"{}\"", DEFAULT_KAS_URL);
    println!(
        "    export ARKAVO_IDENTITY_URL=\"{}\"",
        DEFAULT_IDENTITY_URL
    );
    println!("    export ARKAVO_KAS_TOKEN=\"your-oauth-token\"");
    println!();

    // Step 3: Check for existing token
    println!("Step 3: OAuth Token Status");
    match std::env::var("ARKAVO_KAS_TOKEN") {
        Ok(token) => {
            let masked = if token.len() > 8 {
                format!("{}...{}", &token[..4], &token[token.len() - 4..])
            } else {
                "***".to_string()
            };
            println!("  ✓ Token found: {}", masked);

            let _kas_with_token = KasConfig::production().with_token(token);
            println!("  ✓ KAS configured with token");
            println!();

            println!("You're ready to use KAS operations!");
            println!();
            println!("Next steps:");
            println!("  1. Run encryption example: cargo run --example kas_encrypt_decrypt");
            println!(
                "  2. Test with integration test: cargo test --features kas kas_integration -- --ignored"
            );
        }
        Err(_) => {
            println!("  ⚠ No token found in environment");
            println!();
            println!("OAuth Authentication Flow:");
            println!();
            println!("1. Authenticate with Arkavo Identity Service");
            println!("   Endpoint: {}", DEFAULT_IDENTITY_URL);
            println!();
            println!("2. Obtain OAuth 2.0 access token");
            println!("   The token will be used for KAS authorization");
            println!();
            println!("3. Set token in environment:");
            println!("   export ARKAVO_KAS_TOKEN=\"your-access-token\"");
            println!();
            println!("4. Verify token is valid:");
            println!("   cargo run --example oauth_authentication");
            println!();
            println!("For production usage:");
            println!("  • Implement proper OAuth 2.0 client flow");
            println!("  • Use refresh tokens for long-lived sessions");
            println!("  • Store tokens securely (not in source code)");
            println!("  • Implement token rotation and expiry handling");
        }
    }
    println!();

    // Step 4: Show configuration loading from environment
    println!("Step 4: Loading Configuration");
    let env_config = KasConfig::from_env();
    println!("  KAS URL:      {}", env_config.rewrap_url());
    println!("  Identity URL: {}", env_config.identity_url());
    println!(
        "  Token:        {}",
        if env_config.token.is_some() {
            "Configured"
        } else {
            "Not set"
        }
    );
    println!();

    // Step 5: Show custom configuration
    println!("Step 5: Custom Configuration Example");
    println!("  For development/testing environments:");
    println!();
    println!("  ```rust");
    println!("  let kas = KasConfig::custom(");
    println!("      \"https://dev.kas.example.com/rewrap\",");
    println!("      \"https://dev.identity.example.com\"");
    println!("  ).with_token(dev_token);");
    println!("  ```");
    println!();

    println!("=== Summary ===");
    println!();
    println!("Arkavo uses a two-service architecture:");
    println!("  • Identity Service: OAuth 2.0 authentication and token issuance");
    println!("  • KAS: Key Access Service for TDF policy enforcement");
    println!();
    println!("Both services are available in production:");
    println!("  • Identity: {}", DEFAULT_IDENTITY_URL);
    println!("  • KAS:      {}", DEFAULT_KAS_URL);

    Ok(())
}
