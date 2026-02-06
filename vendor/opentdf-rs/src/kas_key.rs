//! KAS public key retrieval and parsing
//!
//! This module provides functionality to fetch and parse KAS public keys
//! from an OpenTDF platform for RSA and EC key wrapping operations.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "kas-client")]
use reqwest::Client;

// Use aws-lc-rs for RSA public key validation (constant-time, FIPS validated)
#[cfg(feature = "kas-client")]
use aws_lc_rs::rsa::PublicEncryptingKey;

// Use p256 for EC public key validation (re-exported from crate root)
#[cfg(feature = "kas-client")]
use crate::p256::{PublicKey as P256PublicKey, pkcs8::DecodePublicKey};

/// Errors that can occur during KAS public key operations
#[derive(Debug, Error)]
pub enum KasKeyError {
    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("RSA parse error: {0}")]
    RsaParseError(String),

    #[error("EC parse error: {0}")]
    EcParseError(String),

    #[cfg(feature = "kas-client")]
    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
}

/// Response structure from KAS public key endpoint (camelCase format)
#[derive(Debug, Serialize, Deserialize)]
pub struct KasPublicKeyResponse {
    #[serde(rename = "publicKey")]
    pub public_key: String, // PEM-encoded RSA public key
    pub kid: String, // Key ID
}

/// Response structure from KAS public key endpoint (snake_case format)
///
/// Some KAS implementations return keys in snake_case format.
/// This struct handles both formats via serde aliases.
#[derive(Debug, Serialize, Deserialize)]
pub struct KasEcPublicKeyResponse {
    #[serde(alias = "public_key", alias = "publicKey")]
    pub public_key: String, // PEM-encoded EC public key
    pub kid: String, // Key ID (e.g., "ec:secp256r1")
}

/// Fetch the KAS public key from the platform
///
/// # Arguments
///
/// * `kas_url` - Base URL of the KAS service (e.g., "http://localhost:8080/kas")
/// * `http_client` - HTTP client to use for the request
///
/// # Returns
///
/// The PEM-encoded RSA public key as a String
#[cfg(feature = "kas-client")]
pub async fn fetch_kas_public_key(
    kas_url: &str,
    http_client: &Client,
) -> Result<KasPublicKeyResponse, KasKeyError> {
    // Construct the public key endpoint URL
    let endpoint = if kas_url.ends_with("/kas") {
        format!("{}/v2/kas_public_key", kas_url)
    } else if kas_url.ends_with('/') {
        format!("{}v2/kas_public_key", kas_url)
    } else {
        format!("{}/v2/kas_public_key", kas_url)
    };

    // Make the HTTP GET request
    let response = http_client.get(&endpoint).send().await?;

    // Check for HTTP errors
    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();
        return Err(KasKeyError::HttpError(format!(
            "HTTP {}: {}",
            status, error_body
        )));
    }

    // Parse the JSON response
    let key_response: KasPublicKeyResponse = response.json().await?;

    Ok(key_response)
}

/// Parse and validate a PEM-encoded RSA public key
///
/// This function validates that the key is in proper PEM format and can be parsed as an RSA public key.
/// Uses aws-lc-rs for constant-time RSA operations (FIPS validated).
///
/// # Arguments
///
/// * `pem` - PEM-encoded RSA public key string
///
/// # Returns
///
/// The validated PEM string if parsing succeeds
#[cfg(feature = "kas-client")]
pub fn validate_rsa_public_key_pem(pem: &str) -> Result<String, KasKeyError> {
    // Parse PEM to DER
    let parsed_pem = pem::parse(pem)
        .map_err(|e| KasKeyError::RsaParseError(format!("Failed to parse PEM: {}", e)))?;

    // Try to parse the DER as an RSA public key
    PublicEncryptingKey::from_der(parsed_pem.contents()).map_err(|e| {
        KasKeyError::RsaParseError(format!("Failed to parse RSA public key: {:?}", e))
    })?;

    Ok(pem.to_string())
}

/// Fetch the KAS EC public key from the platform
///
/// # Arguments
///
/// * `kas_url` - Base URL of the KAS service (e.g., "http://localhost:8080" or "http://localhost:8080/kas")
/// * `http_client` - HTTP client to use for the request
///
/// # Returns
///
/// The KAS EC public key response containing the PEM-encoded key and key ID
///
/// # Example
///
/// ```no_run
/// use opentdf::kas_key::{fetch_kas_ec_public_key, KasEcPublicKeyResponse};
/// use reqwest::Client;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = Client::new();
/// let response = fetch_kas_ec_public_key("https://kas.example.com", &client).await?;
/// println!("Key ID: {}", response.kid);
/// println!("Public Key: {}", response.public_key);
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "kas-client")]
pub async fn fetch_kas_ec_public_key(
    kas_url: &str,
    http_client: &Client,
) -> Result<KasEcPublicKeyResponse, KasKeyError> {
    // Construct the public key endpoint URL
    let endpoint = if kas_url.ends_with("/kas") {
        format!("{}/v2/kas_public_key", kas_url)
    } else if kas_url.ends_with('/') {
        format!("{}kas/v2/kas_public_key", kas_url)
    } else {
        format!("{}/kas/v2/kas_public_key", kas_url)
    };

    // Make the HTTP GET request
    let response = http_client.get(&endpoint).send().await?;

    // Check for HTTP errors
    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();
        return Err(KasKeyError::HttpError(format!(
            "HTTP {}: {}",
            status, error_body
        )));
    }

    // Parse the JSON response
    let key_response: KasEcPublicKeyResponse = response.json().await?;

    // Validate the key is actually an EC key
    validate_ec_public_key_pem(&key_response.public_key)?;

    Ok(key_response)
}

/// Parse and validate a PEM-encoded EC public key (P-256/secp256r1)
///
/// This function validates that the key is in proper PEM format and can be parsed as a P-256 public key.
/// It handles both:
/// - Proper SPKI format (standard "BEGIN PUBLIC KEY" with algorithm OID)
/// - Raw SEC1 format (some KAS servers return raw EC point bytes in PEM wrapper)
///
/// # Arguments
///
/// * `pem` - PEM-encoded EC public key string
///
/// # Returns
///
/// The validated PEM string if parsing succeeds
#[cfg(feature = "kas-client")]
pub fn validate_ec_public_key_pem(pem: &str) -> Result<String, KasKeyError> {
    use crate::pkcs8::EncodePublicKey;

    // Normalize line endings (handle \r\n from some servers)
    let normalized = pem.replace("\r\n", "\n");

    // First try standard SPKI format
    if let Ok(_key) = P256PublicKey::from_public_key_pem(&normalized) {
        return Ok(normalized);
    }

    // Some KAS servers return raw SEC1 bytes in a PEM wrapper
    // Try to extract and parse as SEC1 format
    let parsed = pem::parse(&normalized)
        .map_err(|e| KasKeyError::EcParseError(format!("Failed to parse PEM: {}", e)))?;

    let der_bytes = parsed.contents();

    // Check if this looks like a raw SEC1 point (starts with 0x04 for uncompressed)
    if der_bytes.first() == Some(&0x04) && der_bytes.len() == 65 {
        // Parse as SEC1 uncompressed point
        let key = P256PublicKey::from_sec1_bytes(der_bytes).map_err(|e| {
            KasKeyError::EcParseError(format!("Failed to parse SEC1 EC public key: {}", e))
        })?;

        // Re-encode as proper SPKI PEM for consistency
        let spki_pem = key
            .to_public_key_pem(Default::default())
            .map_err(|e| KasKeyError::EcParseError(format!("Failed to encode as SPKI: {}", e)))?;

        return Ok(spki_pem.to_string());
    }

    // Check for compressed point (0x02 or 0x03, 33 bytes)
    if (der_bytes.first() == Some(&0x02) || der_bytes.first() == Some(&0x03))
        && der_bytes.len() == 33
    {
        let key = P256PublicKey::from_sec1_bytes(der_bytes).map_err(|e| {
            KasKeyError::EcParseError(format!("Failed to parse compressed SEC1 key: {}", e))
        })?;

        let spki_pem = key
            .to_public_key_pem(Default::default())
            .map_err(|e| KasKeyError::EcParseError(format!("Failed to encode as SPKI: {}", e)))?;

        return Ok(spki_pem.to_string());
    }

    Err(KasKeyError::EcParseError(
        "Unrecognized EC public key format".to_string(),
    ))
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

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_validate_rsa_public_key_pem() {
        // This is a valid test RSA public key
        let valid_pem = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA2dMTGDH4QhhB4aYg4d48
SrapeZD128mpRTRCvbQa5ZaDykVvs8jJ5USJSZErNZ/HVvPyX6gqvQv4HeprAT7i
En445s6sOqObeAYWc5FUatvk3R5KPtgqHOgQIPtXQXnahT7HwvZPCjYoawc2MGax
ejRg20the6MtJHh1K2hUGJ/ic7Hbvk2QMHqYvwjFva4q4Uz3cjiA4RXn4joxm8SE
gLUOPV7pWvv7JzZRRLYiXQAcTb4QvJMIwY997/r228sr+fgYjxK6O0QKPZI2iJ5H
PvJX+E+ceUD7JIZc87FvaA5OqwFUFXqJfYNU4ZE7d6ovRja8JwnErHa+7pEk6KkN
8wIDAQAB
-----END PUBLIC KEY-----"#;

        let result = validate_rsa_public_key_pem(valid_pem);
        assert!(result.is_ok());

        let invalid_pem = "not a valid pem";
        let result = validate_rsa_public_key_pem(invalid_pem);
        assert!(result.is_err());
    }

    #[test]
    fn test_kas_ec_public_key_response_deserialization_snake_case() {
        // Test snake_case format (from actual KAS server)
        let json = r#"{
            "public_key": "-----BEGIN PUBLIC KEY-----\nBP7ISW0/0W1VJIFfv+Zvij25a+WVxx1PtiuVcrRLBkCSdH8YpLGVwwu7cAjwF2YB\nNyzrzkRqm0K4inv5zOIqiLg=\n-----END PUBLIC KEY-----\n",
            "kid": "ec:secp256r1"
        }"#;

        let response: KasEcPublicKeyResponse = serde_json::from_str(json).unwrap();
        assert!(
            response
                .public_key
                .starts_with("-----BEGIN PUBLIC KEY-----")
        );
        assert_eq!(response.kid, "ec:secp256r1");
    }

    #[test]
    fn test_kas_ec_public_key_response_deserialization_camel_case() {
        // Test camelCase format (alternative format)
        let json = r#"{
            "publicKey": "-----BEGIN PUBLIC KEY-----\ntest\n-----END PUBLIC KEY-----\n",
            "kid": "ec:secp256r1"
        }"#;

        let response: KasEcPublicKeyResponse = serde_json::from_str(json).unwrap();
        assert!(
            response
                .public_key
                .starts_with("-----BEGIN PUBLIC KEY-----")
        );
        assert_eq!(response.kid, "ec:secp256r1");
    }

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_validate_ec_public_key_pem() {
        // This is a valid P-256 public key
        let valid_pem = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/shJbT/RbVUkgV+/5m+KPblr5ZXH
HU+2K5VytEsGQJJ0fxiksZXDC7twCPAXZgE3LOvORGqbQriKe/nM4iqIuA==
-----END PUBLIC KEY-----"#;

        let result = validate_ec_public_key_pem(valid_pem);
        assert!(result.is_ok());

        // Test with Windows line endings
        let crlf_pem = "-----BEGIN PUBLIC KEY-----\r\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/shJbT/RbVUkgV+/5m+KPblr5ZXH\r\nHU+2K5VytEsGQJJ0fxiksZXDC7twCPAXZgE3LOvORGqbQriKe/nM4iqIuA==\r\n-----END PUBLIC KEY-----\r\n";
        let result = validate_ec_public_key_pem(crlf_pem);
        assert!(result.is_ok());

        let invalid_pem = "not a valid pem";
        let result = validate_ec_public_key_pem(invalid_pem);
        assert!(result.is_err());
    }
}
