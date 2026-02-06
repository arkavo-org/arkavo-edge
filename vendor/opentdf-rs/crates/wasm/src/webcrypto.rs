//! WebCrypto RSA-OAEP operations via SubtleCrypto
//!
//! This module provides RSA-OAEP key encapsulation using the browser's
//! native WebCrypto API (SubtleCrypto). This eliminates the need for
//! Rust-based RSA implementations in WASM, providing:
//!
//! - Constant-time operations (browser-native)
//! - Hardware acceleration where available
//! - No RUSTSEC-2023-0071 vulnerability exposure
//!
//! # Usage
//!
//! All functions in this module are async because WebCrypto uses Promises.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use js_sys::{Array, ArrayBuffer, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Crypto, CryptoKey, CryptoKeyPair, SubtleCrypto};

/// Get the SubtleCrypto interface from the browser
fn get_subtle_crypto() -> Result<SubtleCrypto, String> {
    let global = js_sys::global();

    // Try window.crypto (browser context)
    if let Ok(crypto) = Reflect::get(&global, &JsValue::from_str("crypto"))
        && !crypto.is_undefined()
    {
        let crypto: Crypto = crypto.unchecked_into();
        return Ok(crypto.subtle());
    }

    // Try self.crypto (Web Worker context)
    if let Ok(self_obj) = Reflect::get(&global, &JsValue::from_str("self"))
        && !self_obj.is_undefined()
        && let Ok(crypto) = Reflect::get(&self_obj, &JsValue::from_str("crypto"))
        && !crypto.is_undefined()
    {
        let crypto: Crypto = crypto.unchecked_into();
        return Ok(crypto.subtle());
    }

    Err("WebCrypto not available in this environment".to_string())
}

/// Create RSA-OAEP algorithm object for WebCrypto
fn rsa_oaep_algorithm() -> Result<Object, String> {
    let algorithm = Object::new();
    Reflect::set(
        &algorithm,
        &JsValue::from_str("name"),
        &JsValue::from_str("RSA-OAEP"),
    )
    .map_err(|_| "Failed to set algorithm name")?;
    Ok(algorithm)
}

/// Create RSA-OAEP import parameters with SHA-1 (for Go SDK compatibility)
fn rsa_oaep_import_params() -> Result<Object, String> {
    let algorithm = Object::new();
    Reflect::set(
        &algorithm,
        &JsValue::from_str("name"),
        &JsValue::from_str("RSA-OAEP"),
    )
    .map_err(|_| "Failed to set algorithm name")?;
    // SHA-1 for OpenTDF Go SDK compatibility
    Reflect::set(
        &algorithm,
        &JsValue::from_str("hash"),
        &JsValue::from_str("SHA-1"),
    )
    .map_err(|_| "Failed to set hash algorithm")?;
    Ok(algorithm)
}

/// Create RSA-OAEP key generation parameters
fn rsa_oaep_keygen_params() -> Result<Object, String> {
    let algorithm = Object::new();
    Reflect::set(
        &algorithm,
        &JsValue::from_str("name"),
        &JsValue::from_str("RSA-OAEP"),
    )
    .map_err(|_| "Failed to set algorithm name")?;
    Reflect::set(
        &algorithm,
        &JsValue::from_str("modulusLength"),
        &JsValue::from_f64(2048.0),
    )
    .map_err(|_| "Failed to set modulus length")?;

    // Public exponent: 65537 (0x010001)
    let public_exponent = Uint8Array::new_with_length(3);
    public_exponent.copy_from(&[0x01, 0x00, 0x01]);
    Reflect::set(
        &algorithm,
        &JsValue::from_str("publicExponent"),
        &public_exponent,
    )
    .map_err(|_| "Failed to set public exponent")?;

    // SHA-1 for OpenTDF Go SDK compatibility
    Reflect::set(
        &algorithm,
        &JsValue::from_str("hash"),
        &JsValue::from_str("SHA-1"),
    )
    .map_err(|_| "Failed to set hash algorithm")?;

    Ok(algorithm)
}

/// Parse PEM to DER bytes
fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    // Remove PEM header/footer and whitespace
    let pem_trimmed = pem.trim();

    // Find the base64 content between headers
    let base64_content = if pem_trimmed.contains("-----BEGIN") {
        let lines: Vec<&str> = pem_trimmed.lines().collect();
        let mut content = String::new();
        let mut in_body = false;

        for line in lines {
            if line.starts_with("-----BEGIN") {
                in_body = true;
                continue;
            }
            if line.starts_with("-----END") {
                break;
            }
            if in_body {
                content.push_str(line.trim());
            }
        }
        content
    } else {
        // Assume it's already base64
        pem_trimmed.replace(['\n', '\r', ' '], "")
    };

    BASE64
        .decode(&base64_content)
        .map_err(|e| format!("Failed to decode PEM base64: {}", e))
}

/// Import RSA public key from SPKI PEM format for encryption
///
/// # Arguments
/// * `pem` - PEM-encoded RSA public key (SPKI format)
///
/// # Returns
/// WebCrypto CryptoKey usable for RSA-OAEP encryption
pub async fn import_rsa_public_key(pem: &str) -> Result<CryptoKey, String> {
    let subtle = get_subtle_crypto()?;

    // Convert PEM to DER
    let der = pem_to_der(pem)?;
    let der_array = Uint8Array::from(der.as_slice());

    // Import as SPKI format
    let algorithm = rsa_oaep_import_params()?;

    let key_usages = Array::new();
    key_usages.push(&JsValue::from_str("encrypt"));

    let promise = subtle
        .import_key_with_object(
            "spki",
            &der_array.buffer(),
            &algorithm,
            false, // not extractable
            &key_usages,
        )
        .map_err(|e| format!("Failed to call importKey: {:?}", e))?;

    let key = JsFuture::from(promise)
        .await
        .map_err(|e| format!("Failed to import public key: {:?}", e))?;

    Ok(key.unchecked_into())
}

/// Import RSA private key from PKCS8 PEM format for decryption
///
/// # Arguments
/// * `pem` - PEM-encoded RSA private key (PKCS8 format)
///
/// # Returns
/// WebCrypto CryptoKey usable for RSA-OAEP decryption
pub async fn import_rsa_private_key(pem: &str) -> Result<CryptoKey, String> {
    let subtle = get_subtle_crypto()?;

    // Convert PEM to DER
    let der = pem_to_der(pem)?;
    let der_array = Uint8Array::from(der.as_slice());

    // Import as PKCS8 format
    let algorithm = rsa_oaep_import_params()?;

    let key_usages = Array::new();
    key_usages.push(&JsValue::from_str("decrypt"));

    let promise = subtle
        .import_key_with_object(
            "pkcs8",
            &der_array.buffer(),
            &algorithm,
            false, // not extractable
            &key_usages,
        )
        .map_err(|e| format!("Failed to call importKey: {:?}", e))?;

    let key = JsFuture::from(promise)
        .await
        .map_err(|e| format!("Failed to import private key: {:?}", e))?;

    Ok(key.unchecked_into())
}

/// RSA-2048 key pair generated by WebCrypto
pub struct WebCryptoRsaKeyPair {
    /// Private key for decryption (CryptoKey)
    pub private_key: CryptoKey,
    /// Public key PEM string (SPKI format)
    pub public_key_pem: String,
}

/// Generate RSA-2048 key pair using WebCrypto
///
/// Creates a new RSA-2048 key pair for secure key exchange with KAS.
/// Uses SHA-1 for OAEP padding (Go SDK compatibility).
///
/// # Returns
/// Key pair with private CryptoKey and PEM-encoded public key
pub async fn generate_rsa_keypair() -> Result<WebCryptoRsaKeyPair, String> {
    let subtle = get_subtle_crypto()?;

    // Generate key pair
    let algorithm = rsa_oaep_keygen_params()?;

    let key_usages = Array::new();
    key_usages.push(&JsValue::from_str("encrypt"));
    key_usages.push(&JsValue::from_str("decrypt"));

    let promise = subtle
        .generate_key_with_object(&algorithm, true, &key_usages)
        .map_err(|e| format!("Failed to call generateKey: {:?}", e))?;

    let key_pair = JsFuture::from(promise)
        .await
        .map_err(|e| format!("Failed to generate key pair: {:?}", e))?;

    let key_pair: CryptoKeyPair = key_pair.unchecked_into();

    // Extract private key
    let private_key: CryptoKey = Reflect::get(&key_pair, &JsValue::from_str("privateKey"))
        .map_err(|_| "Failed to get private key")?
        .unchecked_into();

    // Extract and export public key
    let public_key: CryptoKey = Reflect::get(&key_pair, &JsValue::from_str("publicKey"))
        .map_err(|_| "Failed to get public key")?
        .unchecked_into();

    // Export public key as SPKI DER
    let export_promise = subtle
        .export_key("spki", &public_key)
        .map_err(|e| format!("Failed to call exportKey: {:?}", e))?;

    let exported = JsFuture::from(export_promise)
        .await
        .map_err(|e| format!("Failed to export public key: {:?}", e))?;

    let exported_buffer: ArrayBuffer = exported.unchecked_into();
    let exported_array = Uint8Array::new(&exported_buffer);
    let der_bytes: Vec<u8> = exported_array.to_vec();

    // Convert DER to PEM
    let base64_der = BASE64.encode(&der_bytes);
    let public_key_pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
        base64_der
            .chars()
            .collect::<Vec<char>>()
            .chunks(64)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join("\n")
    );

    Ok(WebCryptoRsaKeyPair {
        private_key,
        public_key_pem,
    })
}

/// RSA-OAEP encrypt (wrap) using WebCrypto
///
/// Encrypts data using RSA-OAEP with SHA-1 padding.
/// This is used to wrap symmetric keys for KAS.
///
/// # Arguments
/// * `public_key` - WebCrypto CryptoKey for encryption
/// * `data` - Data to encrypt (typically a symmetric key)
///
/// # Returns
/// Encrypted ciphertext bytes
pub async fn rsa_oaep_encrypt(public_key: &CryptoKey, data: &[u8]) -> Result<Vec<u8>, String> {
    let subtle = get_subtle_crypto()?;

    let algorithm = rsa_oaep_algorithm()?;
    let data_array = Uint8Array::from(data);

    let promise = subtle
        .encrypt_with_object_and_buffer_source(&algorithm, public_key, &data_array)
        .map_err(|e| format!("Failed to call encrypt: {:?}", e))?;

    let ciphertext = JsFuture::from(promise)
        .await
        .map_err(|e| format!("RSA-OAEP encryption failed: {:?}", e))?;

    let ciphertext_buffer: ArrayBuffer = ciphertext.unchecked_into();
    let ciphertext_array = Uint8Array::new(&ciphertext_buffer);

    Ok(ciphertext_array.to_vec())
}

/// RSA-OAEP decrypt (unwrap) using WebCrypto
///
/// Decrypts data using RSA-OAEP with SHA-1 padding.
/// This is a constant-time operation via browser native implementation.
///
/// # Arguments
/// * `private_key` - WebCrypto CryptoKey for decryption
/// * `ciphertext` - Encrypted data
///
/// # Returns
/// Decrypted plaintext bytes
pub async fn rsa_oaep_decrypt(
    private_key: &CryptoKey,
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    let subtle = get_subtle_crypto()?;

    let algorithm = rsa_oaep_algorithm()?;
    let ciphertext_array = Uint8Array::from(ciphertext);

    let promise = subtle
        .decrypt_with_object_and_buffer_source(&algorithm, private_key, &ciphertext_array)
        .map_err(|e| format!("Failed to call decrypt: {:?}", e))?;

    let plaintext = JsFuture::from(promise)
        .await
        .map_err(|e| format!("RSA-OAEP decryption failed: {:?}", e))?;

    let plaintext_buffer: ArrayBuffer = plaintext.unchecked_into();
    let plaintext_array = Uint8Array::new(&plaintext_buffer);

    Ok(plaintext_array.to_vec())
}

/// Wrap a symmetric key with RSA-OAEP and encode as base64
///
/// Convenience function that imports a PEM public key and wraps a payload key.
///
/// # Arguments
/// * `payload_key` - The symmetric key to wrap (typically 32 bytes for AES-256)
/// * `kas_public_key_pem` - PEM-encoded RSA public key from KAS
///
/// # Returns
/// Base64-encoded wrapped key ready for inclusion in TDF manifest
pub async fn wrap_key_with_rsa_oaep(
    payload_key: &[u8],
    kas_public_key_pem: &str,
) -> Result<String, String> {
    // Import the public key
    let public_key = import_rsa_public_key(kas_public_key_pem).await?;

    // Encrypt the payload key
    let wrapped = rsa_oaep_encrypt(&public_key, payload_key).await?;

    // Base64 encode for storage
    Ok(BASE64.encode(&wrapped))
}

/// Unwrap a symmetric key with RSA-OAEP
///
/// Decrypts a wrapped key using the ephemeral private key.
///
/// # Arguments
/// * `wrapped_key` - The encrypted key bytes
/// * `private_key` - WebCrypto CryptoKey for decryption
///
/// # Returns
/// Decrypted payload key bytes
pub async fn unwrap_key_with_rsa_oaep(
    wrapped_key: &[u8],
    private_key: &CryptoKey,
) -> Result<Vec<u8>, String> {
    rsa_oaep_decrypt(private_key, wrapped_key).await
}
