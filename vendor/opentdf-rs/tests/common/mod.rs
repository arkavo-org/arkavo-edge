//! Common test utilities for opentdf integration tests
//!
//! This module provides shared helper functions to reduce code duplication
//! across integration test files.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

// Re-export commonly used types
pub use opentdf::{
    manifest::TdfManifestExt, AttributeIdentifier, AttributePolicy, AttributeValue, Operator,
    Policy, TdfManifest,
};

/// Platform endpoints for testing
pub const PLATFORM_ENDPOINT: &str = "http://localhost:8080";

/// Test data for encryption/decryption
pub const TEST_PLAINTEXT: &[u8] = b"Hello, OpenTDF! This is test data for encryption.";

/// Create a mock KAS rewrap response for testing
///
/// # Arguments
/// * `wrapped_key_b64` - Base64 encoded wrapped key
/// * `session_public_key_pem` - PEM encoded session public key
pub fn create_mock_rewrap_response(wrapped_key_b64: &str, session_public_key_pem: &str) -> String {
    serde_json::json!({
        "responses": [{
            "policyId": "00000000-0000-0000-0000-000000000000",
            "results": [{
                "keyAccessObjectId": "kao-0",
                "status": "permit",
                "entityWrappedKey": wrapped_key_b64
            }]
        }],
        "sessionPublicKey": session_public_key_pem
    })
    .to_string()
}

/// Create a properly formatted test manifest with a simple policy
///
/// # Arguments
/// * `kas_url` - The KAS URL to use in the manifest
pub fn create_test_manifest_with_policy(kas_url: String) -> TdfManifest {
    // Create a simple policy
    let attr_id = AttributeIdentifier {
        namespace: "example.com".to_string(),
        name: "clearance".to_string(),
    };
    let policy = Policy::new(
        "00000000-0000-0000-0000-000000000000".to_string(), // Valid UUID format (36 chars)
        vec![AttributePolicy::condition(
            attr_id,
            Operator::Equals,
            AttributeValue::String("secret".to_string()),
        )],
        vec![],
    );

    // Create manifest with the policy
    let mut manifest = TdfManifest::new("0.payload".to_string(), kas_url);

    // Set a dummy wrapped key (base64 encoded random bytes)
    manifest.encryption_information.key_access[0].wrapped_key =
        BASE64.encode(b"dummy-wrapped-key-32-bytes-long!");

    // Embed policy
    manifest.set_policy(&policy).unwrap();

    manifest
}

/// Generate a test EC keypair for P-256 curve
///
/// Returns (private_key_pkcs8_der, public_key_sec1_compressed)
#[cfg(feature = "kas-client")]
pub fn generate_p256_keypair() -> (Vec<u8>, Vec<u8>) {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use p256::pkcs8::EncodePrivateKey;
    use p256::SecretKey;
    use rand::rngs::OsRng;

    let secret_key = SecretKey::random(&mut OsRng);
    let public_key = secret_key.public_key();

    // Use PKCS#8 DER for private key (compatible with EC KEM)
    let private_bytes = secret_key.to_pkcs8_der().unwrap().to_bytes().to_vec();
    // Use compressed SEC1 format (33 bytes) per otdfctl gold standard
    let public_bytes = public_key.to_encoded_point(true).as_bytes().to_vec();

    (private_bytes, public_bytes)
}

/// Get KAS configuration from environment variables
///
/// Returns `Some((url, token))` if both `OPENTDF_KAS_URL` and `OPENTDF_TOKEN` are set.
pub fn get_kas_config() -> Option<(String, String)> {
    let url = std::env::var("OPENTDF_KAS_URL").ok()?;
    let token = std::env::var("OPENTDF_TOKEN").ok()?;
    Some((url, token))
}

/// Get access token from environment or default test configuration
///
/// Returns the token if `OPENTDF_TOKEN` environment variable is set.
pub fn get_access_token() -> Option<String> {
    std::env::var("OPENTDF_TOKEN").ok()
}
