//! KAS (Key Access Service) client for WASM
//!
//! This module provides KAS integration for browser environments,
//! enabling secure key wrapping and rewrap protocol.
//!
//! RSA operations use WebCrypto (SubtleCrypto) for:
//! - Constant-time operations (browser-native)
//! - No RUSTSEC-2023-0071 vulnerability exposure

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use opentdf_protocol::kas::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CryptoKey, Request, RequestInit, RequestMode, Response};

use crate::webcrypto;

/// KAS public key response structure
#[derive(Debug, Serialize, Deserialize)]
pub struct KasPublicKeyResponse {
    #[serde(rename = "publicKey")]
    pub public_key: String, // PEM-encoded RSA public key
    pub kid: String, // Key ID
}

/// Fetch the KAS public key from the platform
///
/// Uses browser's Fetch API to retrieve the KAS public key.
///
/// # Arguments
///
/// * `kas_url` - Base URL of the KAS service (e.g., "http://localhost:8080/kas")
///
/// # Returns
///
/// The KAS public key response with PEM-encoded RSA public key and key ID
pub async fn fetch_kas_public_key(kas_url: &str) -> Result<KasPublicKeyResponse, String> {
    // Construct the public key endpoint URL
    let endpoint = if kas_url.ends_with("/kas") {
        format!("{}/v2/kas_public_key", kas_url)
    } else if kas_url.ends_with('/') {
        format!("{}v2/kas_public_key", kas_url)
    } else {
        format!("{}/v2/kas_public_key", kas_url)
    };

    // Create request
    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);

    let request = Request::new_with_str_and_init(&endpoint, &opts)
        .map_err(|e| format!("Failed to create request: {:?}", e))?;

    // Add headers
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(|e| format!("Failed to set headers: {:?}", e))?;

    // Make the request using browser's fetch API
    let window = web_sys::window().ok_or("No window object available")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch failed: {:?}", e))?;

    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| "Response is not a Response object")?;

    // Check status
    if !resp.ok() {
        return Err(format!("HTTP error: {}", resp.status()));
    }

    // Parse JSON
    let json = JsFuture::from(
        resp.json()
            .map_err(|e| format!("Failed to get JSON: {:?}", e))?,
    )
    .await
    .map_err(|e| format!("Failed to parse JSON: {:?}", e))?;

    // Convert to Rust struct
    let key_response: KasPublicKeyResponse = serde_wasm_bindgen::from_value(json)
        .map_err(|e| format!("Failed to deserialize response: {:?}", e))?;

    Ok(key_response)
}

/// Wrap a payload key with RSA-OAEP using KAS public key (async)
///
/// This function wraps a symmetric payload key with an RSA public key using OAEP padding.
/// Uses SHA-1 for compatibility with the OpenTDF Go SDK.
/// Uses WebCrypto for constant-time operations.
///
/// # Arguments
///
/// * `payload_key` - The symmetric key to wrap (typically 32 bytes for AES-256)
/// * `kas_public_key_pem` - PEM-encoded RSA public key from KAS
///
/// # Returns
///
/// Base64-encoded wrapped key ready for inclusion in TDF manifest
pub async fn wrap_key_with_rsa_oaep(
    payload_key: &[u8],
    kas_public_key_pem: &str,
) -> Result<String, String> {
    webcrypto::wrap_key_with_rsa_oaep(payload_key, kas_public_key_pem).await
}

/// RSA-2048 ephemeral key pair for KAS rewrap protocol
///
/// Uses WebCrypto-generated keys for constant-time operations.
pub struct EphemeralRsaKeyPair {
    /// WebCrypto private key for decryption
    pub private_key: CryptoKey,
    /// PEM-encoded public key
    pub public_key_pem: String,
}

/// Generate ephemeral RSA-2048 key pair for KAS communication (async)
///
/// Creates a new RSA-2048 key pair for secure key exchange with KAS.
/// The private key is used to decrypt the wrapped payload key, and the
/// public key is sent to KAS in the rewrap request.
/// Uses WebCrypto for secure key generation.
pub async fn generate_rsa_keypair() -> Result<EphemeralRsaKeyPair, String> {
    let keypair = webcrypto::generate_rsa_keypair().await?;

    Ok(EphemeralRsaKeyPair {
        private_key: keypair.private_key,
        public_key_pem: keypair.public_key_pem,
    })
}

// Protocol structs are now imported from opentdf_protocol::kas

/// Build unsigned rewrap request from TDF manifest
///
/// Extracts policy UUID and key access information from the TDF manifest
/// and constructs a rewrap request for KAS authorization.
pub fn build_rewrap_request(
    manifest: &opentdf::TdfManifest,
    client_public_key_pem: &str,
) -> Result<UnsignedRewrapRequest, String> {
    // Extract policy UUID from base64-encoded policy JSON
    let policy_bytes = BASE64
        .decode(&manifest.encryption_information.policy)
        .map_err(|e| format!("Failed to decode policy: {}", e))?;

    let policy_json: serde_json::Value = serde_json::from_slice(&policy_bytes)
        .map_err(|e| format!("Failed to parse policy JSON: {}", e))?;

    let policy_uuid = policy_json
        .get("uuid")
        .and_then(|v| v.as_str())
        .ok_or("Policy missing 'uuid' field")?
        .to_string();

    // Validate UUID format (36 characters)
    if policy_uuid.len() != 36 {
        return Err(format!("Invalid UUID format: {}", policy_uuid));
    }

    // Build key access objects from manifest
    let key_access_objects: Vec<KeyAccessObjectWrapper> = manifest
        .encryption_information
        .key_access
        .iter()
        .enumerate()
        .map(|(idx, kao)| KeyAccessObjectWrapper {
            key_access_object_id: format!("kao-{}", idx),
            key_access_object: KeyAccessObject {
                key_type: kao.access_type.clone(),
                url: kao.url.clone(),
                protocol: kao.protocol.clone(),
                wrapped_key: kao.wrapped_key.clone(),
                policy_binding: KasPolicyBinding {
                    hash: kao.policy_binding.hash.clone(),
                    algorithm: Some(kao.policy_binding.alg.clone()),
                },
                encrypted_metadata: kao.encrypted_metadata.clone(),
                kid: kao.kid.clone(),
                header: None, // Only used for NanoTDF rewrap requests
                ephemeral_public_key: kao.ephemeral_public_key.clone(),
            },
        })
        .collect();

    Ok(UnsignedRewrapRequest {
        client_public_key: client_public_key_pem.to_string(),
        requests: vec![PolicyRequest {
            algorithm: None, // Standard TDF uses None
            policy: Policy {
                id: policy_uuid,
                body: manifest.encryption_information.policy.clone(),
            },
            key_access_objects,
        }],
    })
}

/// Create signed JWT (ES256) for rewrap request
///
/// Signs the rewrap request with an ephemeral P-256 ECDSA key.
/// The JWT includes the requestBody (as a JSON string), iat, and exp claims.
pub fn create_signed_jwt(request: &UnsignedRewrapRequest) -> Result<String, String> {
    use p256::ecdsa::{SigningKey, signature::Signer};
    use rand::rngs::OsRng;

    // Generate ephemeral P-256 signing key
    let signing_key = SigningKey::random(&mut OsRng);

    // Serialize request body to JSON
    let request_body = serde_json::to_string(request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    // Get current UNIX timestamp using JavaScript Date API
    let now = js_sys::Date::now() as u64 / 1000;

    // Create JWT header and payload
    let header = serde_json::json!({
        "alg": "ES256",
        "typ": "JWT"
    });

    // CRITICAL: requestBody MUST be a string, not a JSON object
    let payload = serde_json::json!({
        "requestBody": request_body,
        "iat": now,
        "exp": now + 60
    });

    // Base64URL encode header and payload
    let header_json =
        serde_json::to_vec(&header).map_err(|e| format!("Failed to encode header: {}", e))?;
    let payload_json =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to encode payload: {}", e))?;

    let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&header_json);
    let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&payload_json);

    // Create signing input
    let signing_input = format!("{}.{}", header_b64, payload_b64);

    // Sign with P-256
    let signature: p256::ecdsa::Signature = signing_key.sign(signing_input.as_bytes());

    // Base64URL encode signature
    let signature_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes());

    // Return complete JWT
    Ok(format!("{}.{}", signing_input, signature_b64))
}

// RewrapResponse, PolicyRewrapResult, KeyAccessRewrapResult are now imported from opentdf_protocol::kas

/// Unwrapped response with key ready for decryption
pub struct UnwrappedResponse {
    pub wrapped_key: Vec<u8>,
}

/// POST rewrap request to KAS using browser Fetch API
///
/// Sends the signed JWT rewrap request to KAS and handles the response.
/// Requires a valid OAuth bearer token for authentication.
pub async fn post_rewrap_request(
    kas_url: &str,
    oauth_token: &str,
    signed_token: &str,
) -> Result<UnwrappedResponse, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response};

    // Construct /v2/rewrap endpoint
    let endpoint = if kas_url.ends_with("/kas") {
        format!("{}/v2/rewrap", kas_url)
    } else if kas_url.ends_with('/') {
        format!("{}v2/rewrap", kas_url)
    } else {
        format!("{}/v2/rewrap", kas_url)
    };

    // Build request body
    let request_body = serde_json::json!({
        "signedRequestToken": signed_token
    });
    let body_str = serde_json::to_string(&request_body)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    // Create request
    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(RequestMode::Cors);
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body_str));

    let request = Request::new_with_str_and_init(&endpoint, &opts)
        .map_err(|e| format!("Failed to create request: {:?}", e))?;

    // Set headers
    request
        .headers()
        .set("Authorization", &format!("Bearer {}", oauth_token))
        .map_err(|e| format!("Failed to set auth header: {:?}", e))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| format!("Failed to set content-type: {:?}", e))?;
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(|e| format!("Failed to set accept header: {:?}", e))?;

    // Make request
    let window = web_sys::window().ok_or("No window object")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch failed: {:?}", e))?;

    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| "Response is not a Response object")?;

    // Handle HTTP errors
    if !resp.ok() {
        let status = resp.status();
        return Err(match status {
            401 => "Authentication failed: Invalid OAuth token".to_string(),
            403 => "Access denied: Policy evaluation failed".to_string(),
            _ => format!("HTTP error: {}", status),
        });
    }

    // Parse response JSON
    let json = JsFuture::from(
        resp.json()
            .map_err(|e| format!("Failed to get JSON: {:?}", e))?,
    )
    .await
    .map_err(|e| format!("Failed to parse JSON: {:?}", e))?;

    let rewrap_resp: RewrapResponse = serde_wasm_bindgen::from_value(json)
        .map_err(|e| format!("Failed to deserialize response: {:?}", e))?;

    // Extract wrapped key from response
    let policy_result = rewrap_resp
        .responses
        .first()
        .ok_or("Empty response from KAS")?;

    let key_result = policy_result
        .results
        .first()
        .ok_or("No key results in response")?;

    // Check access status
    if key_result.status != "permit" {
        let error_msg = key_result
            .error
            .clone()
            .unwrap_or_else(|| "Access denied".to_string());
        return Err(format!("KAS denied access: {}", error_msg));
    }

    // Get wrapped key (try kasWrappedKey first, then legacy entityWrappedKey)
    let wrapped_key_b64 = key_result
        .kas_wrapped_key
        .as_ref()
        .or(key_result.entity_wrapped_key.as_ref())
        .ok_or("Missing wrapped key in response")?;

    let wrapped_key = BASE64
        .decode(wrapped_key_b64)
        .map_err(|e| format!("Failed to decode wrapped key: {}", e))?;

    Ok(UnwrappedResponse { wrapped_key })
}

/// Unwrap payload key using RSA-OAEP (SHA-1) decryption via WebCrypto (async)
///
/// Decrypts the wrapped payload key returned from KAS using the ephemeral
/// RSA private key. Uses OAEP padding with SHA-1 for Go SDK compatibility.
/// Uses WebCrypto for constant-time decryption.
pub async fn unwrap_rsa_oaep(
    wrapped_key: &[u8],
    private_key: &CryptoKey,
) -> Result<Vec<u8>, String> {
    webcrypto::unwrap_key_with_rsa_oaep(wrapped_key, private_key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kas_public_key_response_deserialization() {
        let json = r#"{
            "publicKey": "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA\n-----END PUBLIC KEY-----\n",
            "kid": "r1"
        }"#;

        let response: KasPublicKeyResponse = serde_json::from_str(json).unwrap();
        assert!(
            response
                .public_key
                .starts_with("-----BEGIN PUBLIC KEY-----")
        );
        assert_eq!(response.kid, "r1");
    }
}
