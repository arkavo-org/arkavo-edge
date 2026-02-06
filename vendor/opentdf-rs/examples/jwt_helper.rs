//! JWT Helper for KAS Rewrap Requests
//!
//! This module provides utilities for creating signed JWT tokens (ES256)
//! for OpenTDF KAS rewrap protocol.
//!
//! The KAS server expects a JWT with a "requestBody" claim containing the
//! serialized rewrap request.
//!
//! # Example
//!
//! ```no_run
//! use opentdf_protocol::UnsignedRewrapRequest;
//!
//! // Create your unsigned rewrap request
//! let unsigned_request = UnsignedRewrapRequest {
//!     client_public_key: "...".to_string(),
//!     requests: vec![/* ... */],
//! };
//!
//! // Sign it with your P-256 private key
//! let signed_token = create_signed_jwt(&unsigned_request, &signing_key)?;
//! ```

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use opentdf_protocol::UnsignedRewrapRequest;
use p256::ecdsa::{SigningKey, signature::Signer};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

/// Create a signed JWT (ES256) for a KAS rewrap request
///
/// # Arguments
///
/// * `request` - The unsigned rewrap request to sign
/// * `signing_key` - P-256 ECDSA signing key
///
/// # Returns
///
/// A signed JWT token string in the format: `header.payload.signature`
///
/// # Format
///
/// The JWT contains:
/// - Header: `{"alg": "ES256", "typ": "JWT"}`
/// - Payload: `{"requestBody": "<json>", "iat": <timestamp>, "exp": <timestamp>}`
///
/// # Important
///
/// The `requestBody` claim MUST be a string (JSON-serialized), not a JSON object.
/// This matches the OpenTDF Go SDK's expectation: `token.Get("requestBody").(string)`
pub fn create_signed_jwt(
    request: &UnsignedRewrapRequest,
    signing_key: &SigningKey,
) -> Result<String, Box<dyn std::error::Error>> {
    // Serialize request body to JSON string
    let request_body = serde_json::to_string(request)?;

    // Get current timestamp
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    // Create JWT header
    let header = json!({
        "alg": "ES256",
        "typ": "JWT"
    });

    // Create JWT payload
    // IMPORTANT: requestBody MUST be a string, not a JSON object
    // The KAS server expects: token.Get("requestBody").(string)
    // See platform/service/kas/access/rewrap.go:146-149
    let payload = json!({
        "requestBody": request_body,
        "iat": now,
        "exp": now + 60  // 60 second expiration
    });

    // Encode header and payload as base64url
    let header_b64 = base64url_encode(&serde_json::to_vec(&header)?);
    let payload_b64 = base64url_encode(&serde_json::to_vec(&payload)?);

    // Create signing input
    let signing_input = format!("{}.{}", header_b64, payload_b64);

    // Sign with P-256 key
    let signature: p256::ecdsa::Signature = signing_key.sign(signing_input.as_bytes());

    // Encode signature as base64url
    let signature_b64 = base64url_encode(&signature.to_bytes());

    Ok(format!("{}.{}", signing_input, signature_b64))
}

/// Encode data as base64url (URL-safe, no padding)
fn base64url_encode(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// Generate a new P-256 signing key for JWT creation
///
/// # Returns
///
/// A randomly generated P-256 ECDSA signing key
///
/// # Example
///
/// ```no_run
/// let signing_key = generate_signing_key();
/// ```
pub fn generate_signing_key() -> SigningKey {
    SigningKey::random(&mut rand::rngs::OsRng)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== JWT Helper for OpenTDF KAS ===\n");

    // Generate a signing key
    println!("Generating P-256 signing key...");
    let signing_key = generate_signing_key();
    println!("✓ Signing key generated\n");

    // Create a sample unsigned request
    println!("Creating sample rewrap request...");
    let unsigned_request = opentdf_protocol::UnsignedRewrapRequest {
        client_public_key: "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...\n-----END PUBLIC KEY-----".to_string(),
        requests: vec![],
    };
    println!("✓ Sample request created\n");

    // Sign the request
    println!("Signing request with ES256...");
    let signed_token = create_signed_jwt(&unsigned_request, &signing_key)?;
    println!("✓ JWT token created\n");

    // Display token (truncated for readability)
    let token_preview = if signed_token.len() > 80 {
        format!(
            "{}...{}",
            &signed_token[..40],
            &signed_token[signed_token.len() - 40..]
        )
    } else {
        signed_token.clone()
    };
    println!("Token (truncated): {}", token_preview);
    println!("\nFull token length: {} characters", signed_token.len());

    println!("\n=== Usage in KAS Client ===");
    println!("let kas_client = KasClient::new(kas_url, oauth_token)?;");
    println!("let payload_key = kas_client.rewrap_standard_tdf(&manifest, &signed_token).await?;");

    Ok(())
}
