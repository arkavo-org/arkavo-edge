use opentdf::prelude::*;
use std::collections::HashMap;
use std::io::Cursor;
use wasm_bindgen::prelude::*;

mod kas;
/// WebCrypto RSA-OAEP operations via browser's SubtleCrypto API
pub mod webcrypto;

// Set up panic hook for better error messages in the browser
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Result type for WASM operations
#[wasm_bindgen]
pub struct WasmResult {
    success: bool,
    data: Option<String>,
    error: Option<String>,
}

#[wasm_bindgen]
impl WasmResult {
    #[wasm_bindgen(getter)]
    pub fn success(&self) -> bool {
        self.success
    }

    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Option<String> {
        self.data.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }
}

impl WasmResult {
    fn ok(data: String) -> Self {
        WasmResult {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn err(error: String) -> Self {
        WasmResult {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Create a TDF archive with encrypted data (async)
///
/// This function fetches the KAS public key and wraps the DEK securely.
/// The DEK never leaves the WASM environment.
///
/// # Arguments
/// * `data` - Base64-encoded data to encrypt
/// * `kas_url` - KAS (Key Access Service) URL
/// * `policy_json` - JSON string containing the policy
///
/// # Returns
/// Promise resolving to WasmResult with base64-encoded TDF archive
#[wasm_bindgen]
pub async fn tdf_create(data: String, kas_url: String, policy_json: String) -> WasmResult {
    match _tdf_create_impl(&data, &kas_url, &policy_json).await {
        Ok(result) => WasmResult::ok(result),
        Err(e) => WasmResult::err(e),
    }
}

async fn _tdf_create_impl(data: &str, kas_url: &str, policy_json: &str) -> Result<String, String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    // Step 1: Fetch KAS public key from the server
    let kas_key_response = kas::fetch_kas_public_key(kas_url).await?;

    // Step 2: Decode base64 input data
    let data_bytes = BASE64
        .decode(data)
        .map_err(|e| format!("Failed to decode data: {}", e))?;

    // Step 3: Parse policy
    let policy: Policy =
        serde_json::from_str(policy_json).map_err(|e| format!("Failed to parse policy: {}", e))?;

    // Step 4: Create TDF encryption instance with generated keys
    let tdf_encryption =
        TdfEncryption::new().map_err(|e| format!("Failed to create encryption: {}", e))?;

    // Step 5: Encrypt the data with segments (required for proper decryption)
    let segment_size = 1024 * 1024; // 1MB segments (default)
    let segmented_payload = tdf_encryption
        .encrypt_with_segments(&data_bytes, segment_size)
        .map_err(|e| format!("Failed to encrypt: {}", e))?;

    // Step 6: Wrap the payload key with KAS public key using RSA-OAEP (via WebCrypto)
    let wrapped_key =
        kas::wrap_key_with_rsa_oaep(tdf_encryption.payload_key(), &kas_key_response.public_key)
            .await?;

    // Step 7: Concatenate all encrypted segments
    let ciphertext_bytes: Vec<u8> = segmented_payload
        .segments
        .iter()
        .flat_map(|s| s.iter().copied())
        .collect();

    // Step 8: Create manifest
    let mut manifest = TdfManifest::new("0.payload".to_string(), kas_url.to_string());
    manifest.encryption_information.method.algorithm = "AES-256-GCM".to_string();
    manifest.encryption_information.method.iv = String::new(); // Segments have their own IVs

    // Step 9: Set policy on manifest
    manifest
        .set_policy(&policy)
        .map_err(|e| format!("Failed to set policy: {}", e))?;

    // Step 10: Generate policy binding
    let policy_json_bytes = policy
        .to_json()
        .map_err(|e| format!("Failed to serialize policy: {}", e))?;

    manifest.encryption_information.key_access[0]
        .generate_policy_binding_raw(&policy_json_bytes, tdf_encryption.payload_key())
        .map_err(|e| format!("Failed to generate policy binding: {}", e))?;

    // Step 11: Set wrapped key and kid
    manifest.encryption_information.key_access[0].wrapped_key = wrapped_key;
    manifest.encryption_information.key_access[0].kid = Some(kas_key_response.kid);

    // Step 12: Set segment information in manifest
    use opentdf::manifest::Segment;
    for seg_info in &segmented_payload.segment_info {
        manifest
            .encryption_information
            .integrity_information
            .segments
            .push(Segment {
                hash: seg_info.hash.clone(),
                segment_size: Some(seg_info.plaintext_size),
                encrypted_segment_size: Some(seg_info.encrypted_size),
            });
    }

    // Set segment defaults
    if let Some(first_seg) = segmented_payload.segment_info.first() {
        manifest
            .encryption_information
            .integrity_information
            .segment_size_default = first_seg.plaintext_size;
        manifest
            .encryption_information
            .integrity_information
            .encrypted_segment_size_default = first_seg.encrypted_size;
    }

    // Generate root signature
    manifest
        .encryption_information
        .integrity_information
        .generate_root_signature(&segmented_payload.gmac_tags, tdf_encryption.payload_key())
        .map_err(|e| format!("Failed to generate root signature: {}", e))?;

    // Step 13: Build TDF archive in memory
    let mut builder = TdfArchiveMemoryBuilder::new();
    builder
        .add_entry(&manifest, &ciphertext_bytes, 0)
        .map_err(|e| format!("Failed to add entry: {}", e))?;

    let tdf_bytes = builder
        .finish()
        .map_err(|e| format!("Failed to finish archive: {}", e))?;

    // Step 14: Encode result as base64
    Ok(BASE64.encode(&tdf_bytes))
}

/// Read a TDF archive and return its manifest
///
/// # Arguments
/// * `tdf_data` - Base64-encoded TDF archive
///
/// # Returns
/// JSON string containing the TDF manifest
#[wasm_bindgen]
pub fn tdf_read(tdf_data: &str) -> WasmResult {
    match _tdf_read_impl(tdf_data) {
        Ok(result) => WasmResult::ok(result),
        Err(e) => WasmResult::err(e),
    }
}

fn _tdf_read_impl(tdf_data: &str) -> Result<String, String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    // Decode base64 TDF data
    let tdf_bytes = BASE64
        .decode(tdf_data)
        .map_err(|e| format!("Failed to decode TDF: {}", e))?;

    // Open TDF archive from bytes using Cursor
    let cursor = Cursor::new(tdf_bytes);
    let mut archive = TdfArchive::new(cursor).map_err(|e| format!("Failed to open TDF: {}", e))?;

    // Get entry and manifest
    let entry = archive
        .by_index()
        .map_err(|e| format!("Failed to read TDF entry: {}", e))?;

    // Serialize manifest to JSON
    serde_json::to_string(&entry.manifest)
        .map_err(|e| format!("Failed to serialize manifest: {}", e))
}

/// Decrypt a TDF archive using KAS rewrap protocol (async)
///
/// This performs the complete KAS rewrap flow:
/// 1. Parse TDF manifest and extract policy/key access info
/// 2. Generate ephemeral RSA-2048 key pair
/// 3. Build and sign JWT rewrap request (ES256)
/// 4. POST to KAS /v2/rewrap endpoint with OAuth token
/// 5. Unwrap returned key using RSA-OAEP (SHA-1)
/// 6. Decrypt payload with AES-256-GCM
///
/// # Arguments
/// * `tdf_data` - Base64-encoded TDF archive
/// * `kas_token` - OAuth bearer token for KAS authentication
///
/// # Returns
/// Promise resolving to WasmResult with base64-encoded plaintext
#[wasm_bindgen]
pub async fn tdf_decrypt_with_kas(tdf_data: String, kas_token: String) -> WasmResult {
    match _tdf_decrypt_with_kas_impl(&tdf_data, &kas_token).await {
        Ok(plaintext_b64) => WasmResult::ok(plaintext_b64),
        Err(e) => WasmResult::err(e),
    }
}

async fn _tdf_decrypt_with_kas_impl(tdf_data: &str, kas_token: &str) -> Result<String, String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    // Step 1: Parse TDF archive and extract manifest
    let tdf_bytes = BASE64
        .decode(tdf_data)
        .map_err(|e| format!("Failed to decode TDF: {}", e))?;

    let cursor = Cursor::new(tdf_bytes.clone());
    let mut archive = TdfArchive::new(cursor).map_err(|e| format!("Failed to open TDF: {}", e))?;

    let entry = archive
        .by_index()
        .map_err(|e| format!("Failed to read TDF entry: {}", e))?;

    // Step 2: Generate ephemeral RSA-2048 key pair (via WebCrypto)
    let ephemeral_keypair = kas::generate_rsa_keypair().await?;

    // Step 3: Build unsigned rewrap request
    let unsigned_request =
        kas::build_rewrap_request(&entry.manifest, &ephemeral_keypair.public_key_pem)?;

    // Step 4: Sign request with JWT (ES256)
    let signed_token = kas::create_signed_jwt(&unsigned_request)?;

    // Step 5: POST to KAS /v2/rewrap
    let kas_url = &entry.manifest.encryption_information.key_access[0].url;
    let rewrap_response = kas::post_rewrap_request(kas_url, kas_token, &signed_token).await?;

    // Step 6: Unwrap payload key using RSA-OAEP (via WebCrypto)
    let payload_key =
        kas::unwrap_rsa_oaep(&rewrap_response.wrapped_key, &ephemeral_keypair.private_key).await?;

    // Step 7: Decrypt payload
    let tdf_encryption = TdfEncryption::with_payload_key(&payload_key)
        .map_err(|e| format!("Failed to create decryption context: {}", e))?;

    // Read encrypted payload from archive
    let payload_bytes = entry.payload;

    // Convert segments to the expected format: (plaintext_size, encrypted_size)
    let segment_tuples: Vec<(u64, u64)> = entry
        .manifest
        .encryption_information
        .integrity_information
        .segments
        .iter()
        .map(|seg| {
            (
                seg.segment_size.unwrap_or(0),
                seg.encrypted_segment_size.unwrap_or(0),
            )
        })
        .collect();

    // Decrypt using segment information
    let (plaintext, _gmac_tags) = tdf_encryption
        .decrypt_with_segments(&payload_bytes, &segment_tuples)
        .map_err(|e| format!("Decryption failed: {}", e))?;

    // Step 8: Return base64-encoded plaintext
    Ok(BASE64.encode(&plaintext))
}

/// Create a policy from JSON
///
/// # Arguments
/// * `policy_json` - JSON string containing policy definition
///
/// # Returns
/// JSON string of the validated policy
#[wasm_bindgen]
pub fn policy_create(policy_json: &str) -> WasmResult {
    match _policy_create_impl(policy_json) {
        Ok(result) => WasmResult::ok(result),
        Err(e) => WasmResult::err(e),
    }
}

fn _policy_create_impl(policy_json: &str) -> Result<String, String> {
    let policy: Policy =
        serde_json::from_str(policy_json).map_err(|e| format!("Failed to parse policy: {}", e))?;

    serde_json::to_string(&policy).map_err(|e| format!("Failed to serialize policy: {}", e))
}

/// Evaluate attribute-based access control policy
///
/// # Arguments
/// * `policy_json` - JSON string containing the attribute policy
/// * `attributes_json` - JSON string containing user attributes as key-value pairs
///
/// # Returns
/// Boolean indicating whether access should be granted
#[wasm_bindgen]
pub fn access_evaluate(policy_json: &str, attributes_json: &str) -> WasmResult {
    match _access_evaluate_impl(policy_json, attributes_json) {
        Ok(result) => WasmResult::ok(result.to_string()),
        Err(e) => WasmResult::err(e),
    }
}

fn _access_evaluate_impl(policy_json: &str, attributes_json: &str) -> Result<bool, String> {
    // Parse attribute policy
    let policy: AttributePolicy =
        serde_json::from_str(policy_json).map_err(|e| format!("Failed to parse policy: {}", e))?;

    // Parse user attributes
    let attrs: HashMap<String, serde_json::Value> = serde_json::from_str(attributes_json)
        .map_err(|e| format!("Failed to parse attributes: {}", e))?;

    // Convert to AttributeIdentifier -> AttributeValue map
    let mut user_attrs: HashMap<AttributeIdentifier, AttributeValue> = HashMap::new();
    for (key, value) in attrs {
        let attr_id = AttributeIdentifier::from_string(&key)
            .map_err(|e| format!("Invalid attribute identifier '{}': {}", key, e))?;

        let attr_value = match value {
            serde_json::Value::String(s) => AttributeValue::String(s),
            serde_json::Value::Number(n) => AttributeValue::Number(n.as_f64().unwrap_or(0.0)),
            serde_json::Value::Bool(b) => AttributeValue::Boolean(b),
            serde_json::Value::Array(arr) => {
                // Try to determine array type from first element
                if let Some(first) = arr.first() {
                    match first {
                        serde_json::Value::String(_) => {
                            let strings: Vec<String> = arr
                                .iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect();
                            AttributeValue::StringArray(strings)
                        }
                        serde_json::Value::Number(_) => {
                            let numbers: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                            AttributeValue::NumberArray(numbers)
                        }
                        _ => {
                            return Err(format!(
                                "Unsupported array element type for attribute '{}'",
                                key
                            ));
                        }
                    }
                } else {
                    AttributeValue::StringArray(vec![])
                }
            }
            _ => return Err(format!("Unsupported value type for attribute '{}'", key)),
        };

        user_attrs.insert(attr_id, attr_value);
    }

    // Evaluate policy
    policy
        .evaluate(&user_attrs)
        .map_err(|e| format!("Policy evaluation failed: {}", e))
}

/// Create an attribute identifier from namespace:name format
///
/// # Arguments
/// * `identifier` - String in format "namespace:name"
///
/// # Returns
/// JSON string containing the attribute identifier
#[wasm_bindgen]
pub fn attribute_identifier_create(identifier: &str) -> WasmResult {
    match _attribute_identifier_create_impl(identifier) {
        Ok(result) => WasmResult::ok(result),
        Err(e) => WasmResult::err(e),
    }
}

fn _attribute_identifier_create_impl(identifier: &str) -> Result<String, String> {
    let attr_id = AttributeIdentifier::from_string(identifier)
        .map_err(|e| format!("Failed to create attribute identifier: {}", e))?;

    serde_json::to_string(&attr_id)
        .map_err(|e| format!("Failed to serialize attribute identifier: {}", e))
}

/// Get version information
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn test_version() {
        let v = version();
        assert!(!v.is_empty());
    }

    #[wasm_bindgen_test]
    fn test_attribute_identifier_create() {
        let result = attribute_identifier_create("gov.example:clearance");
        assert!(result.success());
        assert!(result.data().is_some());
    }

    #[wasm_bindgen_test]
    fn test_policy_create() {
        let policy_json = r#"{
            "uuid": "test-uuid",
            "body": {
                "attributes": [],
                "dissem": ["user@example.com"]
            }
        }"#;

        let result = policy_create(policy_json);
        assert!(result.success());
        assert!(result.data().is_some());
    }
}
