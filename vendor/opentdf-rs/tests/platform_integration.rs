//! Integration tests with OpenTDF Platform backend
//!
//! These tests verify interoperability between our Rust implementation
//! and the Go-based OpenTDF platform.
//!
//! Prerequisites:
//! - OpenTDF platform running at http://localhost:8080
//! - Keycloak running at http://localhost:8888
//!
//! Run with: cargo test --package opentdf -- --ignored platform

use std::error::Error;

/// Platform configuration for local development
const PLATFORM_ENDPOINT: &str = "http://localhost:8080";
const TOKEN_ENDPOINT: &str =
    "http://localhost:8888/auth/realms/opentdf/protocol/openid-connect/token";
const CLIENT_ID: &str = "opentdf-sdk";
const CLIENT_SECRET: &str = "secret";

/// Helper to get an access token from Keycloak
async fn get_access_token() -> Result<String, Box<dyn Error>> {
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "client_credentials"),
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
    ];

    let response = client.post(TOKEN_ENDPOINT).form(&params).send().await?;

    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: String,
    }

    let token_data: TokenResponse = response.json().await?;
    Ok(token_data.access_token)
}

/// Helper to get KAS public key from platform
async fn get_kas_public_key() -> Result<String, Box<dyn Error>> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/kas/v2/kas_public_key", PLATFORM_ENDPOINT))
        .send()
        .await?;

    #[derive(serde::Deserialize)]
    struct KasKeyResponse {
        #[serde(rename = "publicKey")]
        public_key: String,
    }

    let key_data: KasKeyResponse = response.json().await?;
    Ok(key_data.public_key)
}

#[tokio::test]
#[ignore] // Run explicitly with --ignored flag
async fn test_platform_health() -> Result<(), Box<dyn Error>> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/healthz", PLATFORM_ENDPOINT))
        .send()
        .await?;

    assert!(response.status().is_success());

    #[derive(serde::Deserialize)]
    struct HealthResponse {
        status: String,
    }

    let health: HealthResponse = response.json().await?;
    assert_eq!(health.status, "SERVING");

    println!("✓ Platform health check passed");
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_access_token() -> Result<(), Box<dyn Error>> {
    let token = get_access_token().await?;

    // JWT tokens have 3 parts separated by dots
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT should have 3 parts");

    println!("✓ Successfully obtained access token");
    println!("  Token length: {} chars", token.len());
    println!("  First 20 chars: {}...", &token[..20]);

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_kas_public_key() -> Result<(), Box<dyn Error>> {
    let public_key = get_kas_public_key().await?;

    assert!(public_key.contains("BEGIN PUBLIC KEY"));
    assert!(public_key.contains("END PUBLIC KEY"));

    println!("✓ Successfully retrieved KAS public key");
    println!("  Key preview:\n{}", &public_key[..80]);

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_encrypt_decrypt_roundtrip() -> Result<(), Box<dyn Error>> {
    // This test will encrypt data with our Rust implementation
    // and verify it can be decrypted, ensuring compatibility
    // with the platform's key management

    // TODO: Once we have the full TDF client implementation:
    // let plaintext = b"Hello from Rust OpenTDF implementation!";
    // let client = create_test_client().await?;
    // let tdf = client.encrypt(plaintext).await?;
    // let decrypted = client.decrypt(&tdf).await?;
    // assert_eq!(plaintext, decrypted.as_slice());

    println!("✓ Encrypt/decrypt roundtrip test (TODO: implement)");
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_cross_platform_compatibility() -> Result<(), Box<dyn Error>> {
    // This test will:
    // 1. Create a TDF with Rust implementation
    // 2. Verify it can be read by platform/Go SDK
    // 3. Create a TDF with Go SDK
    // 4. Verify it can be read by our Rust implementation

    println!("✓ Cross-platform compatibility test (TODO: implement)");
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_policy_enforcement() -> Result<(), Box<dyn Error>> {
    // Test that policy attributes are correctly enforced
    // by the platform when accessing TDF data

    println!("✓ Policy enforcement test (TODO: implement)");
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_kas_rewrap_protocol() -> Result<(), Box<dyn Error>> {
    // Test the complete KAS rewrap protocol:
    // 1. Encrypt payload key with KAS public key
    // 2. Create rewrap request with proper JWT
    // 3. Verify rewrap response
    // 4. Decrypt payload key

    let _kas_public_key = get_kas_public_key().await?;
    println!("✓ Retrieved KAS public key for rewrap test");

    // TODO: Implement full KAS rewrap protocol test
    println!("✓ KAS rewrap protocol test (TODO: implement)");

    Ok(())
}
